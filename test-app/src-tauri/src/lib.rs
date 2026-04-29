use tauri::Emitter;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

#[tauri::command]
fn show_alert(app: tauri::AppHandle) {
    let app_clone = app.clone();
    app.dialog()
        .message("This is a test alert")
        .title("Alert")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            let _ = app_clone.emit("dialog-result", "Alert dismissed");
        });
}

#[tauri::command]
fn show_confirm(app: tauri::AppHandle) {
    let app_clone = app.clone();
    app.dialog()
        .message("Are you sure?")
        .title("Confirm")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancel)
        .show(move |accepted| {
            let result = if accepted { "Confirm OK" } else { "Confirm Cancel" };
            let _ = app_clone.emit("dialog-result", result);
        });
}

#[tauri::command]
fn show_yes_no(app: tauri::AppHandle) {
    let app_clone = app.clone();
    app.dialog()
        .message("Do you agree?")
        .title("Question")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::YesNo)
        .show(move |yes| {
            let result = if yes { "Question Yes" } else { "Question No" };
            let _ = app_clone.emit("dialog-result", result);
        });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            show_alert,
            show_confirm,
            show_yes_no,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
