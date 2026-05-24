use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

/// Push a dialog-result to the WebView two ways at once: the Tauri
/// event channel (`dialog-result` listener) and a direct JS eval into
/// `window.__golemSetDialogResult`. Either alone is sufficient on the
/// happy path, but Tauri's event listener has a registration race on
/// cold start — the listener's IPC handshake can lose to a dialog that
/// opens and dismisses very quickly under sweep load, dropping the
/// event. The eval path runs unconditionally as soon as the WebView is
/// alive, so the rendered "lastResult" row always updates.
fn deliver_dialog_result(app: &tauri::AppHandle, payload: &'static str) {
    let _ = app.emit("dialog-result", payload);
    if let Some(window) = app.get_webview_window("main") {
        let script = format!(
            "window.__golemSetDialogResult && window.__golemSetDialogResult({})",
            serde_json::to_string(payload).unwrap_or_else(|_| "''".to_string())
        );
        let _ = window.eval(&script);
    }
}

#[tauri::command]
fn show_alert(app: tauri::AppHandle) {
    let app_clone = app.clone();
    app.dialog()
        .message("This is a test alert")
        .title("Alert")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            deliver_dialog_result(&app_clone, "Alert dismissed");
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
            deliver_dialog_result(&app_clone, result);
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
            deliver_dialog_result(&app_clone, result);
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
