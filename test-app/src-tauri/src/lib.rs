use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

#[tauri::command]
fn show_alert(app: tauri::AppHandle) {
    // Show non-blocking — the dialog displays and the command returns.
    // The dialog remains visible until the user dismisses it.
    app.dialog()
        .message("This is a test alert")
        .title("Alert")
        .kind(MessageDialogKind::Info)
        .show(|_| {});
}

#[tauri::command]
fn show_confirm(app: tauri::AppHandle) {
    app.dialog()
        .message("Are you sure?")
        .title("Confirm")
        .kind(MessageDialogKind::Warning)
        .show(|_confirmed| {});
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![show_alert, show_confirm])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
