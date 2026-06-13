//! Application entry point and event loop.
//!
//! Wires Revue's App with AppStore to produce the main event loop.

use anyhow::Context;
use revue::prelude::*;
use revue::event::Key;

use crate::store::app_store::{AppStore, Route};
use crate::screen::{HomeScreen, SessionScreen};

/// Run the AgenDao TUI application.
pub fn run_app() -> anyhow::Result<()> {
    let store = AppStore::new();

    let mut app = App::builder()
        .mouse_capture(true)
        .build();

    let view = RootView { store: store.clone() };
    app.run_with_handler(view, move |key_event, view| {
        view.handle_key(&key_event.key)
    })
    .context("agendao TUI runtime exited with error")
}

/// Root view — routes to Home or Session via AppStore.route
struct RootView {
    store: AppStore,
}

impl RootView {
    fn handle_key(&mut self, key: &Key) -> bool {
        match key {
            Key::Char('q') | Key::Escape => {
                self.store.request_exit();
                true
            }
            Key::Char('h') => {
                self.store.navigate(Route::Home);
                true
            }
            _ => false,
        }
    }

    fn render_status_bar(&self, ctx: &mut RenderContext) {
        let route_label = self.store.route.get().as_str();
        let status_text = format!(" agendao | [{}] | q/Esc: quit | h: home ", route_label);

        let bar_y = ctx.area.height.saturating_sub(1);
        for x in 0..ctx.area.width {
            ctx.draw_text(x, bar_y, " ", Color::rgb(30, 32, 44));
        }
        ctx.draw_text(0, bar_y, &status_text, Color::rgb(169, 177, 214));
    }
}

impl View for RootView {
    fn render(&self, ctx: &mut RenderContext) {
        let route = self.store.route.get();

        match &route {
            Route::Home => {
                HomeScreen { store: &self.store }.render(ctx);
            }
            Route::Session { session_id } => {
                SessionScreen {
                    session_id: session_id.clone(),
                }
                .render(ctx);
            }
        }

        self.render_status_bar(ctx);
    }
}
