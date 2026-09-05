mod assets;
mod theme;
mod workspace;

use std::sync::OnceLock;

use clap::Parser;
use gpui::{
    App, AppContext as _, Bounds, KeyBinding, WindowAppearance, WindowBounds, WindowOptions, point,
    px, size,
};
use gpui_component::{Root, theme::Theme};
use time::OffsetDateTime;
use tracing_subscriber::EnvFilter;

use crate::{
    application::TaskApplication,
    cli::Cli,
    infrastructure::{AppPaths, AppSettings, InstanceLock},
};

use workspace::{
    CloseDetailAction, CommandPaletteAction, DeleteAction, MoveDownAction, MoveUpAction,
    NewTaskAction, RedoAction, SearchAction, SelectAllAction, ToggleDoneAction, UndoAction,
    Workspace,
};

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::resolve(cli.data_dir.as_deref())?;
    initialize_logging(&paths);
    let instance_lock = InstanceLock::acquire(&paths.lock)?;
    let first_run = !paths.settings.exists();
    let settings = AppSettings::load(&paths.settings);
    let worker = TaskApplication::start(&paths.database)?;
    if !worker.is_read_only() {
        worker.purge_expired_trash(OffsetDateTime::now_utc())?;
        workspace::schedule_maintenance(worker.clone(), paths.clone());
    }
    let snapshot = worker.load()?;

    let window_width = settings.window.width;
    let window_height = settings.window.height;
    let app = gpui::Application::new().with_assets(assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        let shortcuts = if cfg!(target_os = "macos") {
            vec![
                KeyBinding::new("cmd-n", NewTaskAction, Some("HodoQ")),
                KeyBinding::new("cmd-f", SearchAction, Some("HodoQ")),
                KeyBinding::new("up", MoveUpAction, Some("HodoQ")),
                KeyBinding::new("down", MoveDownAction, Some("HodoQ")),
                KeyBinding::new("cmd-enter", ToggleDoneAction, Some("HodoQ")),
                KeyBinding::new("cmd-backspace", DeleteAction, Some("HodoQ")),
                KeyBinding::new("escape", CloseDetailAction, Some("HodoQ")),
                KeyBinding::new("cmd-k", CommandPaletteAction, Some("HodoQ")),
                KeyBinding::new("cmd-z", UndoAction, Some("HodoQ")),
                KeyBinding::new("cmd-shift-z", RedoAction, Some("HodoQ")),
                KeyBinding::new("cmd-a", SelectAllAction, Some("HodoQ")),
            ]
        } else {
            vec![
                KeyBinding::new("ctrl-n", NewTaskAction, Some("HodoQ")),
                KeyBinding::new("ctrl-f", SearchAction, Some("HodoQ")),
                KeyBinding::new("up", MoveUpAction, Some("HodoQ")),
                KeyBinding::new("down", MoveDownAction, Some("HodoQ")),
                KeyBinding::new("ctrl-enter", ToggleDoneAction, Some("HodoQ")),
                KeyBinding::new("delete", DeleteAction, Some("HodoQ")),
                KeyBinding::new("escape", CloseDetailAction, Some("HodoQ")),
                KeyBinding::new("ctrl-k", CommandPaletteAction, Some("HodoQ")),
                KeyBinding::new("ctrl-z", UndoAction, Some("HodoQ")),
                KeyBinding::new("ctrl-shift-z", RedoAction, Some("HodoQ")),
                KeyBinding::new("ctrl-a", SelectAllAction, Some("HodoQ")),
            ]
        };
        cx.bind_keys(shortcuts);
        Theme::change(WindowAppearance::Dark, None, cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let window_size = size(px(window_width), px(window_height));
        let bounds = match (settings.window.x, settings.window.y) {
            (Some(x), Some(y)) => {
                let candidate = Bounds::new(point(px(x), px(y)), window_size);
                if cx
                    .displays()
                    .iter()
                    .any(|display| candidate.intersects(&display.bounds()))
                {
                    candidate
                } else {
                    Bounds::centered(None, window_size, cx)
                }
            }
            _ => Bounds::centered(None, window_size, cx),
        };
        let window_bounds = if settings.window.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                app_id: Some("dev.okawak.hodoq".to_owned()),
                window_min_size: Some(size(px(900.0), px(600.0))),
                ..Default::default()
            },
            move |window, cx| {
                let workspace = cx.new(|cx| {
                    Workspace::new(
                        worker,
                        snapshot,
                        paths,
                        settings,
                        instance_lock,
                        first_run,
                        window,
                        cx,
                    )
                });
                let close_workspace = workspace.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    close_workspace
                        .update(cx, |workspace, cx| workspace.should_close(cx))
                        .unwrap_or(true)
                });
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .expect("failed to open HodoQ window");
        cx.activate(true);
    });
    Ok(())
}

fn initialize_logging(paths: &AppPaths) {
    let file_appender = tracing_appender::rolling::daily(&paths.logs, "hodoq.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("hodoq=info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}
