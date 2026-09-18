/// Send a desktop notification for a highlight or PM.
pub(crate) fn notify_message(from: &str, text: &str, is_pm: bool) {
    let summary = if is_pm {
        format!("PM from {from}")
    } else {
        format!("{from} mentioned you")
    };
    if let Err(e) = notify_rust::Notification::new()
        .summary(&summary)
        .body(text)
        .timeout(5000)
        .show()
    {
        tracing::warn!(error = %e, "desktop notification failed");
    }
}
