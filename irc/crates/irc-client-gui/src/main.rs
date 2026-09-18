mod app;
mod notifications;
mod theme;
mod views;

use app::IrcApp;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    iced::application("IRC Client", IrcApp::update, IrcApp::view)
        .subscription(IrcApp::subscription)
        .theme(IrcApp::theme)
        .run_with(IrcApp::new)
}
