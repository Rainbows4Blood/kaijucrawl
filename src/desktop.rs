//! Thin Tauri IPC boundary; domain work remains in the controller and Rust engine.
use crate::{
    controller::{AccessResult, AuthDraft, Controller, FailurePage, ProfileChoice, Snapshot},
    paths::LocalPaths,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Manager, State};
type Backend<'a> = State<'a, Arc<Controller>>;

#[tauri::command]
async fn warhammer_books(
    state: Backend<'_>,
    query: crate::library::Query,
) -> Result<crate::library::Page, String> {
    state.warhammer_books(query).await
}
#[tauri::command]
async fn warhammer_book(
    state: Backend<'_>,
    game: crate::library::Game,
    target: String,
    product_id: String,
) -> Result<Option<crate::library::Details>, String> {
    state.warhammer_book(game, target, product_id).await
}

#[tauri::command]
async fn snapshot(state: Backend<'_>) -> Result<Snapshot, String> {
    Ok(state.snapshot().await)
}
#[tauri::command]
async fn check_storage(state: Backend<'_>) -> Result<(), String> {
    state.initialize().await
}
#[tauri::command]
async fn select_target(state: Backend<'_>, selected: bool) -> Result<(), String> {
    state.select_target(selected).await
}
#[tauri::command]
async fn browser_profiles(state: Backend<'_>) -> Result<Vec<ProfileChoice>, String> {
    state.profiles().await
}
#[tauri::command]
async fn save_auth(state: Backend<'_>, draft: AuthDraft) -> Result<(), String> {
    state.save_auth(draft).await
}
#[tauri::command]
async fn clear_auth(state: Backend<'_>) -> Result<(), String> {
    state.clear_auth().await
}
#[tauri::command]
async fn test_access(
    state: Backend<'_>,
    product_id: Option<String>,
) -> Result<AccessResult, String> {
    state.test_access(product_id).await
}
#[tauri::command]
async fn start(state: Backend<'_>, reprocess: bool) -> Result<(), String> {
    state.start(reprocess).await
}
#[tauri::command]
async fn stop(state: Backend<'_>) -> Result<(), String> {
    state.stop().await;
    Ok(())
}
#[tauri::command]
async fn resume(state: Backend<'_>) -> Result<(), String> {
    state.resume().await
}
#[tauri::command]
async fn failure_page(state: Backend<'_>, after: i64) -> Result<FailurePage, String> {
    state.failures(after).await
}
#[tauri::command]
async fn start_analysis(state: Backend<'_>) -> Result<(), String> {
    state.start_analysis().await
}
#[tauri::command]
async fn start_discovery_analysis(state: Backend<'_>, rerun_all: bool) -> Result<(), String> {
    state.start_discovery_analysis(rerun_all).await
}
#[tauri::command]
async fn start_system_analysis(
    state: Backend<'_>,
    workspace: crate::analysis::AnalysisWorkspace,
    rerun_all: bool,
) -> Result<(), String> {
    state.start_system_analysis(workspace, rerun_all).await
}
#[tauri::command]
async fn start_dnd_classification(state: Backend<'_>, reclassify: bool) -> Result<(), String> {
    state.start_dnd_classification(reclassify).await
}
#[tauri::command]
async fn start_dnd_editions(state: Backend<'_>, reclassify: bool) -> Result<(), String> {
    state.start_dnd_editions(reclassify).await
}
#[tauri::command]
async fn start_dnd_settings(state: Backend<'_>, reclassify: bool) -> Result<(), String> {
    state.start_dnd_settings(reclassify).await
}
#[tauri::command]
async fn start_dnd_extraction(
    state: Backend<'_>,
    kind: crate::dnd_context::Kind,
    reanalyze: bool,
) -> Result<(), String> {
    state.start_dnd_extraction(kind, reanalyze).await
}
#[tauri::command]
async fn analysis_overview(state: Backend<'_>) -> Result<crate::analysis::Overview, String> {
    state.analysis_overview().await
}
#[tauri::command]
async fn start_earth_editions(
    state: Backend<'_>,
    kind: crate::earth_editions::Kind,
    reclassify: bool,
) -> Result<(), String> {
    state.start_earth_editions(kind, reclassify).await
}
#[tauri::command]
async fn start_earth_extraction(
    state: Backend<'_>,
    kind: crate::earth_context::Kind,
    reanalyze: bool,
) -> Result<(), String> {
    state.start_earth_extraction(kind, reanalyze).await
}
#[tauri::command]
async fn start_solo_classification(state: Backend<'_>, reclassify: bool) -> Result<(), String> {
    state.start_solo_classification(reclassify).await
}
#[tauri::command]
async fn start_classification(state: Backend<'_>, reclassify: bool) -> Result<(), String> {
    state.start_classification(reclassify).await
}
#[tauri::command]
async fn start_player_analysis(state: Backend<'_>, reanalyze: bool) -> Result<(), String> {
    state.start_player_analysis(reanalyze).await
}
#[tauri::command]
async fn cancel_analysis(state: Backend<'_>) -> Result<(), String> {
    state.cancel_analysis().await;
    Ok(())
}
#[tauri::command]
async fn test_decisions(state: Backend<'_>) -> Result<crate::decisions::Decision, String> {
    state.test_decisions().await
}
#[derive(Default)]
struct Closing {
    started: AtomicBool,
    finished: AtomicBool,
}
fn close(app: tauri::AppHandle) {
    let closing = app.state::<Arc<Closing>>().inner().clone();
    if closing.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let controller = app.state::<Arc<Controller>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        controller.shutdown().await;
        closing.finished.store(true, Ordering::SeqCst);
        app.exit(0);
    });
}
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    close(app);
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_at(LocalPaths::current()?, true)
}
/// Disposable desktop fixtures use the same IPC and window without production data.
#[doc(hidden)]
pub fn run_at(paths: LocalPaths, production_load: bool) -> Result<(), Box<dyn std::error::Error>> {
    paths.ensure()?;
    let controller = Controller::new(paths.clone(), production_load);
    let boot = controller.clone();
    let app = tauri::Builder::default()
        .manage(controller)
        .manage(Arc::new(Closing::default()))
        .invoke_handler(tauri::generate_handler![
            snapshot,
            check_storage,
            select_target,
            browser_profiles,
            save_auth,
            clear_auth,
            test_access,
            start,
            stop,
            resume,
            failure_page,
            warhammer_books,
            warhammer_book,
            start_analysis,
            start_discovery_analysis,
            start_system_analysis,
            start_classification,
            start_solo_classification,
            start_dnd_classification,
            start_dnd_editions,
            start_dnd_settings,
            start_dnd_extraction,
            start_earth_extraction,
            start_earth_editions,
            analysis_overview,
            start_player_analysis,
            cancel_analysis,
            test_decisions,
            quit
        ])
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("KaijuCrawl")
            .inner_size(1240.0, 820.0)
            .min_inner_size(760.0, 560.0)
            .data_directory(paths.root().join("webview"))
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_navigation(|url| {
                url.scheme() == "tauri"
                    || url.host_str() == Some("tauri.localhost")
                    || (cfg!(debug_assertions)
                        && url.host_str() == Some("127.0.0.1")
                        && url.port() == Some(1420))
            })
            .build()?;
            tauri::async_runtime::spawn(async move {
                let _ = boot.initialize().await;
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    close(handle);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                close(window.app_handle().clone());
            }
        })
        .build(tauri::generate_context!())?;
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !app.state::<Arc<Closing>>().finished.load(Ordering::SeqCst)
        {
            api.prevent_exit();
            close(app.clone());
        }
    });
    Ok(())
}
