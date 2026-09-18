use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Fill};

use crate::app::{DisplayMessage, Msg};

/// Render the central scrollback area showing messages.
pub(crate) fn view(messages: &[DisplayMessage]) -> Element<'_, Msg> {
    let mut col = column![].spacing(1);

    for msg in messages {
        let prefix = if msg.is_action {
            format!("[{}] * {}", msg.timestamp, msg.from)
        } else {
            format!("[{}] <{}>", msg.timestamp, msg.from)
        };

        let line = row![text(prefix).size(12), text(&msg.text).size(12),].spacing(4);

        col = col.push(line);
    }

    container(scrollable(col).height(Fill).width(Fill).anchor_bottom())
        .width(Fill)
        .height(Fill)
        .padding(4)
        .into()
}
