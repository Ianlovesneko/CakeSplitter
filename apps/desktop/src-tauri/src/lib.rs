mod selection;

use std::path::PathBuf;

use cakesplitter_core::{CoreError, inspect_package as inspect_native};
use cakesplitter_desktop_runtime::{
    DesktopPreferences, EngineError, InspectionSummary, ProcessingPlan, StartupRecoveryReport,
    StoreError, TaskEngine, TaskSnapshot,
};
use cakesplitter_format::{FORMAT_VERSION, validate_portable_filename};
use selection::{SelectionKind, SelectionRegistry, SelectionSummary, manifest_from_selection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

struct DesktopState {
    engine: TaskEngine,
    selections: SelectionRegistry,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
    application_version: &'static str,
    format_version: &'static str,
    platform: &'static str,
    automatic_updates: bool,
    telemetry: bool,
    background_service: bool,
    signed_build: bool,
    startup_recovery: StartupRecoveryReport,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloseAction {
    Check,
    KeepOpen,
    CancelTasks,
    InterruptAndExit,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandError {
    code: String,
    message: String,
}

impl CommandError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into().chars().take(80).collect(),
            message: message.into().chars().take(2_000).collect(),
        }
    }
}

impl From<CoreError> for CommandError {
    fn from(value: CoreError) -> Self {
        Self::new(value.code(), privacy_safe_core_message(&value))
    }
}

impl From<EngineError> for CommandError {
    fn from(value: EngineError) -> Self {
        let code = match &value {
            EngineError::Core(error) => error.code(),
            EngineError::InvalidTaskId => "invalid_task_id",
            EngineError::NotActive => "task_not_active",
            EngineError::PauseTimeout => "pause_timeout",
            EngineError::InvalidSliceSize => "invalid_slice_size",
            EngineError::SliceLimit => "resource_limit",
            EngineError::InsufficientSpace { .. } => "insufficient_space",
            EngineError::InvalidState => "invalid_task_state",
            EngineError::QueueUnavailable => "queue_unavailable",
            EngineError::TasksStopping => "tasks_stopping",
            EngineError::Store(StoreError::QueueCapacityReached { .. }) => "queue_capacity_reached",
            EngineError::Store(StoreError::RecoveryCapacityExceeded { .. }) => {
                "recovery_capacity_exceeded"
            }
            EngineError::Store(StoreError::TaskMetadataTooLarge { .. }) => {
                "task_metadata_too_large"
            }
            EngineError::Store(StoreError::TaskPlanTooLarge) => {
                "task_plan_exceeds_supported_bounds"
            }
            EngineError::Store(_) => "task_store_error",
        };
        let message = match &value {
            EngineError::Core(error) => privacy_safe_core_message(error),
            EngineError::Store(StoreError::Io { source, .. }) => {
                format!("The local task store could not be accessed: {source}")
            }
            EngineError::Store(StoreError::QueueCapacityReached { .. }) => {
                "The task queue is full. Finish or remove nonterminal work before adding another task."
                    .to_owned()
            }
            EngineError::Store(StoreError::RecoveryCapacityExceeded { .. }) => {
                "Startup recovery stopped because the persisted nonterminal task limit was exceeded."
                    .to_owned()
            }
            EngineError::Store(StoreError::TaskMetadataTooLarge { .. }) => {
                "The task metadata exceeds the supported local limit.".to_owned()
            }
            EngineError::Store(StoreError::TaskPlanTooLarge) => {
                "The task plan exceeds the supported Slice limit.".to_owned()
            }
            EngineError::Store(_) => "The local task store operation failed safely.".to_owned(),
            _ => value.to_string(),
        };
        Self::new(code, message)
    }
}

impl From<cakesplitter_format::ManifestError> for CommandError {
    fn from(value: cakesplitter_format::ManifestError) -> Self {
        Self::from(CoreError::from(value))
    }
}

fn privacy_safe_core_message(error: &CoreError) -> String {
    match error {
        CoreError::Io { source, .. } => format!("A local filesystem operation failed: {source}"),
        CoreError::InvalidInput(_) => "The selected input is not a regular file.".to_owned(),
        CoreError::Collision(_) => "The planned output already exists.".to_owned(),
        CoreError::StagedIdentityChanged(_) => {
            "The incomplete output identity changed before publication.".to_owned()
        }
        CoreError::StagedContentChanged(_) => {
            "The incomplete output content changed before publication.".to_owned()
        }
        CoreError::AtomicFinalizationUnsupported(_) => {
            "Atomic no-replace publication is unavailable for this destination.".to_owned()
        }
        CoreError::UnsafeFilesystemPath(_) => {
            "The selected filesystem path is unsafe or ambiguous.".to_owned()
        }
        CoreError::DestinationIdentityChanged(_) => {
            "The selected output destination changed or could not be proven stable.".to_owned()
        }
        CoreError::PackageIdentityChanged(_) => {
            "The selected Cake Package changed or could not be proven stable. Select it again."
                .to_owned()
        }
        CoreError::PackageEnumerationLimit { .. } => {
            "The selected Cake Package exceeds a supported local resource limit.".to_owned()
        }
        _ => error.to_string(),
    }
}

#[tauri::command]
fn get_runtime_info(state: State<'_, DesktopState>) -> RuntimeInfo {
    RuntimeInfo {
        application_version: env!("CARGO_PKG_VERSION"),
        format_version: FORMAT_VERSION,
        platform: "windows-x64",
        automatic_updates: false,
        telemetry: false,
        background_service: false,
        signed_build: false,
        startup_recovery: state.engine.startup_recovery_report(),
    }
}

#[tauri::command]
fn choose_source_file(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<SelectionSummary>, CommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Select a Cake to Split")
        .blocking_pick_file();
    selected
        .map(into_path)
        .transpose()?
        .map(|path| state.selections.issue_file(path, SelectionKind::SourceFile))
        .transpose()
}

#[tauri::command]
fn choose_manifest_file(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<SelectionSummary>, CommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Select a Cake Manifest")
        .add_filter("Cake Manifest", &["cake.json", "json"])
        .blocking_pick_file();
    selected
        .map(into_path)
        .transpose()?
        .map(|path| {
            state
                .selections
                .issue_file(path, SelectionKind::ManifestFile)
        })
        .transpose()
}

#[tauri::command]
fn choose_package_folder(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<SelectionSummary>, CommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Select a Cake Package Folder")
        .blocking_pick_folder();
    selected
        .map(into_path)
        .transpose()?
        .map(|path| {
            state
                .selections
                .issue_directory(path, SelectionKind::PackageFolder)
        })
        .transpose()
}

#[tauri::command]
fn choose_output_folder(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<SelectionSummary>, CommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Select an Output Folder")
        .blocking_pick_folder();
    selected
        .map(into_path)
        .transpose()?
        .map(|path| {
            state
                .selections
                .issue_directory(path, SelectionKind::OutputFolder)
        })
        .transpose()
}

#[tauri::command]
fn choose_output_file(
    app: AppHandle,
    state: State<'_, DesktopState>,
    suggested_name: String,
) -> Result<Option<SelectionSummary>, CommandError> {
    validate_portable_filename(&suggested_name)?;
    let selected = app
        .dialog()
        .file()
        .set_title("Choose Rebuilt Cake Output")
        .set_file_name(&suggested_name)
        .blocking_save_file();
    selected
        .map(into_path)
        .transpose()?
        .map(|path| state.selections.issue_output_file(path))
        .transpose()
}

#[tauri::command]
fn choose_slice_files(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Option<SelectionSummary>, CommandError> {
    let selected = app
        .dialog()
        .file()
        .set_title("Select Cake Slices")
        .add_filter("Cake Slice", &["slice"])
        .blocking_pick_files();
    selected
        .map(|paths| {
            paths
                .into_iter()
                .map(into_path)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .map(|paths| state.selections.issue_slices(paths))
        .transpose()
}

#[tauri::command]
fn plan_split(
    state: State<'_, DesktopState>,
    source_token: String,
    output_token: String,
    slice_size: u64,
) -> Result<ProcessingPlan, CommandError> {
    let source = state
        .selections
        .resolve_one(&source_token, &[SelectionKind::SourceFile])?;
    let output = state
        .selections
        .resolve_one(&output_token, &[SelectionKind::OutputFolder])?;
    Ok(state.engine.plan_split(&source, &output, slice_size)?)
}

#[tauri::command]
fn preview_merge(
    state: State<'_, DesktopState>,
    package_token: String,
) -> Result<InspectionSummary, CommandError> {
    let manifest = manifest_from_selection(&state.selections, &package_token)?;
    let inspection = inspect_native(&manifest, false, &Default::default())?;
    Ok(inspection.into())
}

#[tauri::command]
fn enqueue_split(
    state: State<'_, DesktopState>,
    source_token: String,
    output_token: String,
    slice_size: u64,
) -> Result<TaskSnapshot, CommandError> {
    let source = state
        .selections
        .resolve_one(&source_token, &[SelectionKind::SourceFile])?;
    let output = state
        .selections
        .resolve_one(&output_token, &[SelectionKind::OutputFolder])?;
    Ok(state.engine.enqueue_split(source, output, slice_size)?)
}

#[tauri::command]
fn enqueue_merge(
    state: State<'_, DesktopState>,
    package_token: String,
    output_token: String,
) -> Result<TaskSnapshot, CommandError> {
    let manifest = manifest_from_selection(&state.selections, &package_token)?;
    let output = state
        .selections
        .resolve_one(&output_token, &[SelectionKind::OutputFile])?;
    Ok(state.engine.enqueue_merge(manifest, output)?)
}

#[tauri::command]
fn inspect_package(
    state: State<'_, DesktopState>,
    package_token: String,
    verify_hashes: bool,
) -> Result<TaskSnapshot, CommandError> {
    let manifest = manifest_from_selection(&state.selections, &package_token)?;
    Ok(state.engine.enqueue_inspect(manifest, verify_hashes)?)
}

#[tauri::command]
fn verify_package(
    state: State<'_, DesktopState>,
    package_token: String,
) -> Result<TaskSnapshot, CommandError> {
    let manifest = manifest_from_selection(&state.selections, &package_token)?;
    Ok(state.engine.enqueue_verify(manifest)?)
}

#[tauri::command]
fn list_tasks(state: State<'_, DesktopState>) -> Result<Vec<TaskSnapshot>, CommandError> {
    Ok(state.engine.list_tasks()?)
}

#[tauri::command]
fn pause_task(
    state: State<'_, DesktopState>,
    task_id: String,
) -> Result<TaskSnapshot, CommandError> {
    Ok(state.engine.pause_task(&task_id)?)
}

#[tauri::command]
fn resume_task(
    state: State<'_, DesktopState>,
    task_id: String,
) -> Result<TaskSnapshot, CommandError> {
    Ok(state.engine.resume_task(&task_id)?)
}

#[tauri::command]
fn cancel_task(
    state: State<'_, DesktopState>,
    task_id: String,
) -> Result<TaskSnapshot, CommandError> {
    Ok(state.engine.cancel_task(&task_id)?)
}

#[tauri::command]
fn retry_task(
    state: State<'_, DesktopState>,
    task_id: String,
) -> Result<TaskSnapshot, CommandError> {
    Ok(state.engine.retry_task(&task_id)?)
}

#[tauri::command]
fn remove_task(state: State<'_, DesktopState>, task_id: String) -> Result<(), CommandError> {
    Ok(state.engine.remove_task(&task_id)?)
}

#[tauri::command]
fn clear_selected_task(
    state: State<'_, DesktopState>,
    task_id: String,
) -> Result<(), CommandError> {
    Ok(state.engine.remove_task(&task_id)?)
}

#[tauri::command]
fn clear_all_tasks(state: State<'_, DesktopState>) -> Result<(), CommandError> {
    Ok(state.engine.clear_all()?)
}

#[tauri::command]
fn get_settings(state: State<'_, DesktopState>) -> Result<DesktopPreferences, CommandError> {
    Ok(state.engine.preferences()?)
}

#[tauri::command]
fn update_settings(
    state: State<'_, DesktopState>,
    settings: DesktopPreferences,
) -> Result<DesktopPreferences, CommandError> {
    Ok(state.engine.save_preferences(&settings)?)
}

#[tauri::command]
fn prepare_app_close(
    app: AppHandle,
    state: State<'_, DesktopState>,
    action: CloseAction,
) -> Result<Vec<String>, CommandError> {
    match action {
        CloseAction::Check | CloseAction::KeepOpen => Ok(state.engine.active_tasks()),
        CloseAction::CancelTasks => {
            let active = state.engine.active_tasks();
            for task_id in &active {
                state.engine.cancel_task(task_id)?;
            }
            Ok(active)
        }
        CloseAction::InterruptAndExit => {
            let interrupted = state.engine.interrupt_all()?;
            let ids = interrupted.into_iter().map(|task| task.id).collect();
            app.exit(0);
            Ok(ids)
        }
    }
}

fn into_path(path: tauri_plugin_dialog::FilePath) -> Result<PathBuf, CommandError> {
    path.into_path().map_err(|_| {
        CommandError::new(
            "invalid_selection",
            "The native dialog did not return a local filesystem path.",
        )
    })
}

fn register_drop(window: &tauri::Window, paths: &[PathBuf]) {
    let Some(state) = window.try_state::<DesktopState>() else {
        return;
    };
    if paths.len() == 1 {
        let path = paths[0].clone();
        let summary = if path.is_dir() {
            state
                .selections
                .issue_directory(path, SelectionKind::PackageFolder)
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".cake.json"))
        {
            state
                .selections
                .issue_file(path, SelectionKind::ManifestFile)
        } else {
            state.selections.issue_file(path, SelectionKind::SourceFile)
        };
        match summary {
            Ok(summary) => {
                let _ = window.emit("native-drop", summary);
            }
            Err(error) => {
                let _ = window.emit("native-drop-error", error);
            }
        }
    } else if !paths.is_empty() {
        match state.selections.issue_slices(paths.to_vec()) {
            Ok(summary) => {
                let _ = window.emit("native-drop", summary);
            }
            Err(error) => {
                let _ = window.emit("native-drop-error", error);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data = app.path().app_local_data_dir()?;
            let handle = app.handle().clone();
            let engine = TaskEngine::open(&app_data, env!("CARGO_PKG_VERSION"), move |task| {
                let _ = handle.emit("task-update", task);
            })
            .map_err(|error| {
                let error: Box<dyn std::error::Error> = Box::new(error);
                tauri::Error::Setup(error.into())
            })?;
            app.manage(DesktopState {
                engine,
                selections: SelectionRegistry::default(),
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                let state = window.state::<DesktopState>();
                let active = state.engine.active_tasks();
                if !active.is_empty() {
                    api.prevent_close();
                    let _ = window.emit("close-requested-with-active-tasks", active);
                }
            }
            WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                register_drop(window, paths);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_runtime_info,
            choose_source_file,
            choose_manifest_file,
            choose_package_folder,
            choose_output_folder,
            choose_output_file,
            choose_slice_files,
            plan_split,
            preview_merge,
            enqueue_split,
            enqueue_merge,
            inspect_package,
            verify_package,
            list_tasks,
            pause_task,
            resume_task,
            cancel_task,
            retry_task,
            remove_task,
            clear_selected_task,
            clear_all_tasks,
            get_settings,
            update_settings,
            prepare_app_close
        ])
        .run(tauri::generate_context!())
        .expect("CakeSplitter Desktop runtime failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_errors_bound_diagnostics_and_redact_private_paths() {
        let private = PathBuf::from(r"C:\Users\Private Name\secret.bin");
        for error in [
            CoreError::Collision(private.clone()),
            CoreError::UnsafeFilesystemPath(private.clone()),
            CoreError::DestinationIdentityChanged(private.clone()),
            CoreError::Io {
                path: private.clone(),
                source: std::io::Error::other("device failure"),
            },
        ] {
            let error = CommandError::from(error);
            assert!(!error.message.contains("Private Name"));
            assert!(!error.message.contains("secret.bin"));
        }

        let bounded = CommandError::new("x".repeat(200), "y".repeat(4_000));
        assert_eq!(bounded.code.chars().count(), 80);
        assert_eq!(bounded.message.chars().count(), 2_000);
    }

    #[test]
    fn production_capability_remains_local_and_narrow() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["app"]["security"]["capabilities"],
            serde_json::json!(["main"])
        );
        assert_eq!(
            config["build"]["devUrl"],
            serde_json::json!("http://127.0.0.1:1420")
        );
        assert!(config["app"]["windows"][0].get("url").is_none());
        let csp = config["app"]["security"]["csp"].as_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("object-src 'none'"));
        assert!(!csp.contains("https:"));

        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:event:default", "allow-cakesplitter-commands"])
        );

        let permissions = include_str!("../permissions/cakesplitter.toml");
        for forbidden in ["shell:", "http:", "fs:", "process:"] {
            assert!(!permissions.contains(forbidden));
        }
    }

    #[test]
    fn admission_failures_keep_specific_bounded_ipc_codes() {
        let capacity = CommandError::from(EngineError::Store(StoreError::QueueCapacityReached {
            maximum: 64,
        }));
        assert_eq!(capacity.code, "queue_capacity_reached");
        assert!(capacity.message.contains("queue is full"));

        let recovery =
            CommandError::from(EngineError::Store(StoreError::RecoveryCapacityExceeded {
                actual: 65,
                maximum: 64,
            }));
        assert_eq!(recovery.code, "recovery_capacity_exceeded");
        assert!(!recovery.message.contains("65"));
    }
}
