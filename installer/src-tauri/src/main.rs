#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use lyra_installer_core::InstallConfig;
use lyra_installer_core::service::{ExecutionEvent, ExecutionRequest};
use lyra_installer_core::storage::{
    DiscoveryBackend, GuidedChoice, InstallPlan, PlanBuilder, StorageSnapshot,
    SystemDiscoveryBackend,
};
use tauri::Emitter;

#[derive(serde::Serialize)]
struct TimezoneEntry {
    name: String,
    latitude: f64,
    longitude: f64,
}

fn parse_zone_coordinate(value: &str, degree_digits: usize) -> Option<f64> {
    let sign = if value.starts_with('-') { -1.0 } else { 1.0 };
    let digits = value.get(1..)?;
    let degrees: f64 = digits.get(..degree_digits)?.parse().ok()?;
    let minutes: f64 = digits.get(degree_digits..degree_digits + 2)?.parse().ok()?;
    let seconds: f64 = if digits.len() >= degree_digits + 4 {
        digits
            .get(degree_digits + 2..degree_digits + 4)?
            .parse()
            .ok()?
    } else {
        0.0
    };
    Some(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
}

#[tauri::command]
fn list_timezones() -> Result<Vec<TimezoneEntry>, String> {
    let source = fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .or_else(|_| fs::read_to_string("/usr/share/zoneinfo/zone.tab"))
        .map_err(|error| format!("não foi possível ler a base de fusos horários: {error}"))?;
    let mut zones = Vec::new();
    for line in source.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() < 3 {
            continue;
        }
        let coordinates = fields[1];
        let split = coordinates
            .get(1..)
            .and_then(|rest| rest.find(['+', '-']).map(|index| index + 1));
        let Some(split) = split else { continue };
        let (latitude, longitude) = coordinates.split_at(split);
        let (Some(latitude), Some(longitude)) = (
            parse_zone_coordinate(latitude, 2),
            parse_zone_coordinate(longitude, 3),
        ) else {
            continue;
        };
        zones.push(TimezoneEntry {
            name: fields[2].to_string(),
            latitude,
            longitude,
        });
    }
    zones.push(TimezoneEntry {
        name: "UTC".into(),
        latitude: 51.48,
        longitude: 0.0,
    });
    zones.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(zones)
}

/// Read-only: lists disks, RAID arrays and LVM volumes currently visible to
/// the live session. Never touches the disk — planning and execution are
/// separate steps until the user confirms the summary screen.
#[tauri::command]
fn discover_storage() -> Result<StorageSnapshot, String> {
    SystemDiscoveryBackend
        .snapshot()
        .map_err(|error| error.to_string())
}

/// Dry-run only, same guarantee as `PlanBuilder::build` itself: no I/O, safe
/// to call from the unprivileged frontend as the user builds a target choice
/// on the storage step. `snapshot` is the one the UI already fetched via
/// `discover_storage` rather than a fresh read, so the plan is built against
/// exactly what the user was shown. Takes the full `GuidedChoice` — both the
/// "whole disk, direct layout" and "new RAID array, direct layout" screens
/// send one of these; `volume_layer` stays `Direct` from every screen so
/// far (no LVM authoring UI), but nothing here assumes that.
#[tauri::command]
fn plan_install(snapshot: StorageSnapshot, choice: GuidedChoice) -> Result<InstallPlan, String> {
    PlanBuilder::new(&snapshot)
        .build(&choice)
        .map_err(|error| error.0.join(" · "))
}

/// Runs the real `InstallConfig::validate()` against whatever the wizard has
/// collected so far — no I/O, same dry-run guarantee as `plan_install`.
/// This is the summary step's own check, not a duplicate of it: page 4's
/// client-side `validate()` in `app.js` only covers full name/username/
/// hostname/password, so this is what actually catches an invalid
/// `timezone`/`locale` (there's no client-side rule for those). Errors
/// still don't cross the privilege boundary — only `execute_plan` does that,
/// after this validation succeeds and the user accepts the destructive
/// confirmation on the summary screen.
#[tauri::command]
fn validate_install_config(config: InstallConfig) -> Result<(), Vec<&'static str>> {
    config.validate()
}

/// Path the polkit action (`io.lyra.Installer.execute-plan`) is scoped to —
/// keep both in sync. Development builds may use a locally staged service;
/// the live image receives this path from the `lyra-installer` RPM.
const SERVICE_PATH: &str = "/usr/libexec/lyra-installer-service";
const SYSTEMCTL_PATH: &str = "/usr/bin/systemctl";
const TRACE_FILENAME: &str = "lyra-installer-trace.log";
const EVIDENCE_FILENAME: &str = "lyra-installer-result.json";

fn redacted_config(config: &InstallConfig) -> serde_json::Value {
    serde_json::json!({
        "locale": config.locale,
        "timezone": config.timezone,
        "keyboard_layout": config.keyboard_layout,
        "hostname": config.hostname,
        "full_name": config.full_name,
        "username": config.username,
        "password": "<redacted>"
    })
}

/// Creates the diagnostic log as the unprivileged desktop user, before
/// pkexec is launched. The privileged service therefore never opens a path
/// controlled by the live user, and the resulting file remains easy for the
/// tester to copy from their own home directory.
fn create_install_trace(request: &ExecutionRequest) -> Result<(File, PathBuf), String> {
    let home = env::var_os("HOME").ok_or("HOME não está definido")?;
    let home = PathBuf::from(home);
    if !home.is_absolute() || !home.is_dir() {
        return Err(format!("diretório HOME inválido: {}", home.display()));
    }

    let path = home.join(TRACE_FILENAME);
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (!metadata.file_type().is_file() || metadata.file_type().is_symlink())
    {
        return Err(format!(
            "o caminho do trace não é um arquivo regular: {}",
            path.display()
        ));
    }

    let mut trace = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("não foi possível criar {}: {error}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("não foi possível proteger {}: {error}", path.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let source = fs::read_to_string("/usr/share/lyra-installer/build-source.txt")
        .unwrap_or_else(|error| format!("indisponível: {error}"));
    let summary = serde_json::json!({
        "choice": &request.choice,
        "plan": &request.plan,
        "config": redacted_config(&request.config)
    });

    writeln!(trace, "Lyra Installer trace")
        .and_then(|_| writeln!(trace, "timestamp_unix={timestamp}"))
        .and_then(|_| writeln!(trace, "installer_version={}", env!("CARGO_PKG_VERSION")))
        .and_then(|_| writeln!(trace, "build_source={}", source.trim()))
        .and_then(|_| writeln!(trace, "request={summary}"))
        .and_then(|_| trace.flush())
        .map_err(|error| format!("não foi possível escrever {}: {error}", path.display()))?;

    Ok((trace, path))
}

fn append_trace(trace: &mut File, line: &str) {
    let _ = writeln!(trace, "{line}");
    let _ = trace.flush();
}

fn write_installer_evidence(
    home: &std::path::Path,
    events: &[ExecutionEvent],
    service_succeeded: bool,
) -> Result<PathBuf, String> {
    let output = home.join(EVIDENCE_FILENAME);
    let temporary = home.join(format!(".{EVIDENCE_FILENAME}.tmp-{}", std::process::id()));
    let source = fs::read_to_string("/usr/share/lyra-installer/build-source.txt")
        .unwrap_or_else(|error| format!("indisponível: {error}"));
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let failed_event = events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::Failed { .. }));
    let completed_event = events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::Completed));
    let passed = service_succeeded && completed_event && !failed_event;
    let document = serde_json::json!({
        "schema": 1,
        "status": if passed { "passed" } else { "failed" },
        "mode": "installer",
        "generated_at_unix": timestamp,
        "installer_version": env!("CARGO_PKG_VERSION"),
        "build_source": source.trim(),
        "checks": [
            {
                "id": "service-exit",
                "status": if service_succeeded { "passed" } else { "failed" },
                "detail": if service_succeeded {
                    "privileged service exited successfully"
                } else {
                    "privileged service exited unsuccessfully"
                }
            },
            {
                "id": "completed-event",
                "status": if completed_event && !failed_event { "passed" } else { "failed" },
                "detail": if completed_event && !failed_event {
                    "terminal Completed event received"
                } else if failed_event {
                    "terminal Failed event received"
                } else {
                    "terminal Completed event not received"
                }
            }
        ],
        "events": events,
    });

    let result = (|| -> Result<(), String> {
        let mut stream = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("não foi possível criar {}: {error}", temporary.display()))?;
        serde_json::to_writer_pretty(&mut stream, &document)
            .map_err(|error| format!("não foi possível serializar a evidência: {error}"))?;
        writeln!(stream).map_err(|error| error.to_string())?;
        stream.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &output)
            .map_err(|error| format!("não foi possível publicar {}: {error}", output.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| output)
}

/// Reuses the packaged application artwork inside the static frontend instead
/// of maintaining a second copy that can drift from the RPM/window icon.
#[tauri::command]
fn installer_logo() -> Vec<u8> {
    include_bytes!("../icons/256x256.png").to_vec()
}

const PREFERRED_WINDOW_WIDTH: u32 = 1180;
const PREFERRED_WINDOW_HEIGHT: u32 = 880;
const WINDOW_HORIZONTAL_MARGIN: u32 = 48;
const WINDOW_VERTICAL_MARGIN: u32 = 72;

fn fitted_window_size(
    current: tauri::PhysicalSize<u32>,
    work_area: tauri::PhysicalSize<u32>,
) -> tauri::PhysicalSize<u32> {
    let available_width = work_area.width.saturating_sub(WINDOW_HORIZONTAL_MARGIN);
    let available_height = work_area.height.saturating_sub(WINDOW_VERTICAL_MARGIN);

    tauri::PhysicalSize::new(
        current
            .width
            .min(PREFERRED_WINDOW_WIDTH)
            .min(available_width),
        current
            .height
            .min(PREFERRED_WINDOW_HEIGHT)
            .min(available_height),
    )
}

fn fitted_window_position(
    fitted_inner: tauri::PhysicalSize<u32>,
    current_inner: tauri::PhysicalSize<u32>,
    current_outer: tauri::PhysicalSize<u32>,
    current_inner_position: tauri::PhysicalPosition<i32>,
    current_outer_position: tauri::PhysicalPosition<i32>,
    work_area_position: tauri::PhysicalPosition<i32>,
    work_area_size: tauri::PhysicalSize<u32>,
) -> tauri::PhysicalPosition<i32> {
    // GTK can report the client size as the outer size while GNOME's
    // server-side title bar is still visible. The difference between the
    // client and frame positions preserves that missing decoration extent.
    let left_decoration = current_inner_position
        .x
        .saturating_sub(current_outer_position.x)
        .max(0) as u32;
    let top_decoration = current_inner_position
        .y
        .saturating_sub(current_outer_position.y)
        .max(0) as u32;
    let horizontal_decoration = current_outer
        .width
        .saturating_sub(current_inner.width)
        .max(left_decoration.saturating_mul(2));
    let vertical_decoration = current_outer
        .height
        .saturating_sub(current_inner.height)
        .max(top_decoration);
    let fitted_outer = tauri::PhysicalSize::new(
        fitted_inner.width.saturating_add(horizontal_decoration),
        fitted_inner.height.saturating_add(vertical_decoration),
    );

    tauri::PhysicalPosition::new(
        work_area_position.x.saturating_add(
            work_area_size
                .width
                .saturating_sub(fitted_outer.width)
                .saturating_div(2) as i32,
        ),
        work_area_position.y.saturating_add(
            work_area_size
                .height
                .saturating_sub(fitted_outer.height)
                .saturating_div(2) as i32,
        ),
    )
}

/// Keeps the entire installer inside the monitor work area after the VM or
/// desktop changes resolution. The margin accounts for GNOME's title bar and
/// leaves the wizard footer reachable instead of allowing it below the screen.
/// Positioning is explicit because Tauri's generic `center()` can omit GNOME's
/// server-side title bar and leave the decorated window visibly too low.
#[tauri::command]
fn fit_window_to_monitor(window: tauri::WebviewWindow) -> Result<(), String> {
    let current_monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?;
    let monitor = match current_monitor {
        Some(monitor) => monitor,
        None => window
            .primary_monitor()
            .map_err(|error| error.to_string())?
            .ok_or("não foi possível detectar o monitor atual")?,
    };
    let work_area = monitor.work_area();
    let current_inner = window.inner_size().map_err(|error| error.to_string())?;
    let current_outer = window.outer_size().map_err(|error| error.to_string())?;
    let current_inner_position = window.inner_position().map_err(|error| error.to_string())?;
    let current_outer_position = window.outer_position().map_err(|error| error.to_string())?;
    let size = fitted_window_size(current_inner, work_area.size);
    let position = fitted_window_position(
        size,
        current_inner,
        current_outer,
        current_inner_position,
        current_outer_position,
        work_area.position,
        work_area.size,
    );

    window.set_size(size).map_err(|error| error.to_string())?;
    window
        .set_position(position)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Moves the blocking child-process/pipe loop off the Tauri command thread.
/// Keeping it in a synchronous command made WebKit's window stop repainting,
/// so GNOME repeatedly offered an "Aguarde" response while the installation
/// itself continued normally in the background.
#[tauri::command]
async fn execute_plan(
    request: ExecutionRequest,
    window: tauri::WebviewWindow,
) -> Result<Vec<ExecutionEvent>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        execute_plan_blocking(request, |event| {
            // Losing the window must not interrupt an installation that has
            // already crossed the destructive boundary. The complete event
            // list is still returned as a fallback when the command ends.
            let _ = window.emit("installation-event", event);
        })
    })
    .await
    .map_err(|error| format!("a tarefa de instalação foi interrompida: {error}"))?
}

/// Requests a reboot through systemd/logind as the active live-session user.
/// Keeping the command unprivileged lets the desktop's normal polkit policy
/// decide whether the local session may restart the machine.
fn system_restart_command() -> Command {
    let mut command = Command::new(SYSTEMCTL_PATH);
    command.arg("reboot");
    command
}

#[tauri::command]
async fn restart_system() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let status = system_restart_command()
            .status()
            .map_err(|error| format!("não foi possível executar {SYSTEMCTL_PATH}: {error}"))?;

        if status.success() {
            Ok(())
        } else {
            Err(format!("o pedido de reinicialização terminou com {status}"))
        }
    })
    .await
    .map_err(|error| format!("a solicitação de reinicialização foi interrompida: {error}"))?
}

/// Launches the privileged service via `pkexec` for the duration of this one
/// call only — never the whole UI (see `docs/installer-architecture.md`).
/// Sends the confirmed plan on stdin and forwards each stdout event while the
/// service is running. The complete list is also returned once the child exits
/// so the frontend can verify the terminal result and recover if an emitted
/// window event was missed.
fn execute_plan_blocking(
    request: ExecutionRequest,
    mut on_event: impl FnMut(&ExecutionEvent),
) -> Result<Vec<ExecutionEvent>, String> {
    let (mut trace, trace_path) = create_install_trace(&request)?;
    append_trace(&mut trace, "frontend=starting privileged service");

    let mut child = Command::new("pkexec")
        .arg(SERVICE_PATH)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            let message = format!("não foi possível iniciar o serviço privilegiado: {error}");
            append_trace(&mut trace, &format!("frontend_error={message}"));
            message
        })?;

    let payload = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    {
        let mut stdin = child.stdin.take().ok_or("stdin do serviço indisponível")?;
        writeln!(stdin, "{payload}").map_err(|error| error.to_string())?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or("stdout do serviço indisponível")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("stderr do serviço indisponível")?;
    let stderr_reader = thread::spawn(move || {
        let mut message = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut message);
        message
    });

    let mut events = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        append_trace(&mut trace, &format!("service_event={line}"));
        let event = serde_json::from_str::<ExecutionEvent>(&line)
            .map_err(|error| format!("evento inesperado do serviço: {error}"))?;
        on_event(&event);
        events.push(event);
    }

    let status = child.wait().map_err(|error| error.to_string())?;
    let stderr = stderr_reader
        .join()
        .unwrap_or_else(|_| "não foi possível ler o erro do serviço".to_string());
    if !stderr.trim().is_empty() {
        append_trace(&mut trace, &format!("service_stderr={}", stderr.trim()));
    }
    append_trace(&mut trace, &format!("service_status={status}"));
    append_trace(&mut trace, &format!("trace_path={}", trace_path.display()));
    let failed_event = events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::Failed { .. }));
    let completed_event = events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::Completed));

    let home = trace_path
        .parent()
        .ok_or("o trace do instalador não possui diretório pai")?;
    match write_installer_evidence(home, &events, status.success()) {
        Ok(path) => append_trace(&mut trace, &format!("release_evidence={}", path.display())),
        Err(error) => append_trace(&mut trace, &format!("evidence_error={error}")),
    }

    if !status.success() && !failed_event {
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("o serviço privilegiado terminou com {status}")
        } else {
            format!("o serviço privilegiado não iniciou: {detail}")
        });
    }
    if status.success() && !completed_event {
        return Err("o serviço terminou sem confirmar a conclusão".to_string());
    }

    Ok(events)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_timezones,
            discover_storage,
            plan_install,
            validate_install_config,
            installer_logo,
            fit_window_to_monitor,
            execute_plan,
            restart_system
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar o Lyra Installer");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_installer_logo_is_a_png() {
        let logo = installer_logo();
        assert!(logo.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn diagnostic_config_never_contains_the_password() {
        let config = InstallConfig {
            password: "segredo-que-nao-pode-ir-ao-log".to_string(),
            ..InstallConfig::default()
        };
        let summary = redacted_config(&config);
        assert_eq!(summary["password"], "<redacted>");
        assert!(!summary.to_string().contains(&config.password));
    }

    #[test]
    fn installer_evidence_is_structured_private_and_contains_no_request_secret() {
        let directory = std::env::temp_dir().join(format!(
            "lyra-installer-evidence-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let events = vec![ExecutionEvent::Started, ExecutionEvent::Completed];

        let output = write_installer_evidence(&directory, &events, true).unwrap();
        let metadata = fs::metadata(&output).unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();

        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(document["status"], "passed");
        assert_eq!(document["mode"], "installer");
        assert_eq!(document["events"].as_array().unwrap().len(), 2);
        assert!(!document.to_string().contains("password"));
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn installer_evidence_cannot_pass_after_a_failed_event() {
        let directory = std::env::temp_dir().join(format!(
            "lyra-installer-failed-evidence-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let events = vec![
            ExecutionEvent::Started,
            ExecutionEvent::Failed {
                step: "extrair rootfs".to_string(),
                message: "squashfs corrompido".to_string(),
            },
        ];

        let output = write_installer_evidence(&directory, &events, false).unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();

        assert_eq!(document["status"], "failed");
        assert_eq!(document["checks"][0]["status"], "failed");
        assert_eq!(document["checks"][1]["status"], "failed");
        assert_eq!(
            document["checks"][1]["detail"],
            "terminal Failed event received"
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn fitted_window_leaves_room_for_desktop_chrome() {
        let fitted = fitted_window_size(
            tauri::PhysicalSize::new(1180, 880),
            tauri::PhysicalSize::new(1024, 728),
        );

        assert_eq!(fitted, tauri::PhysicalSize::new(976, 656));
    }

    #[test]
    fn fitted_window_does_not_enlarge_a_user_resized_window() {
        let fitted = fitted_window_size(
            tauri::PhysicalSize::new(800, 600),
            tauri::PhysicalSize::new(1920, 1040),
        );

        assert_eq!(fitted, tauri::PhysicalSize::new(800, 600));
    }

    #[test]
    fn fitted_window_position_accounts_for_a_gnome_title_bar() {
        let position = fitted_window_position(
            tauri::PhysicalSize::new(1180, 880),
            tauri::PhysicalSize::new(1180, 880),
            // This is the misleading outer size observed under GTK: it does
            // not include the 47 px frame visible in the position delta.
            tauri::PhysicalSize::new(1180, 880),
            tauri::PhysicalPosition::new(369, 170),
            tauri::PhysicalPosition::new(369, 123),
            tauri::PhysicalPosition::new(0, 40),
            tauri::PhysicalSize::new(1920, 1040),
        );

        assert_eq!(position, tauri::PhysicalPosition::new(370, 96));
    }

    #[test]
    fn fitted_window_position_uses_a_correctly_reported_outer_size() {
        let position = fitted_window_position(
            tauri::PhysicalSize::new(1180, 880),
            tauri::PhysicalSize::new(1180, 880),
            tauri::PhysicalSize::new(1180, 927),
            tauri::PhysicalPosition::new(369, 170),
            tauri::PhysicalPosition::new(369, 123),
            tauri::PhysicalPosition::new(0, 40),
            tauri::PhysicalSize::new(1920, 1040),
        );

        assert_eq!(position, tauri::PhysicalPosition::new(370, 96));
    }

    #[test]
    fn restart_uses_systemd_without_a_shell() {
        let command = system_restart_command();

        assert_eq!(command.get_program(), SYSTEMCTL_PATH);
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["reboot"]);
    }
}
