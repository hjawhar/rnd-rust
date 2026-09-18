use bytes::Bytes;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill};

use crate::app::Msg;

/// A single entry from a channel LIST reply.
#[derive(Debug, Clone)]
pub(crate) struct ListEntry {
    pub(crate) channel: Bytes,
    pub(crate) user_count: u32,
    pub(crate) topic: String,
}

/// Render the channel list overlay.
pub(crate) fn view<'a>(
    entries: &'a [ListEntry],
    filter: &'a str,
    loading: bool,
) -> Element<'a, Msg> {
    let filter_input = text_input("Filter channels…", filter)
        .on_input(Msg::ListFilterChanged)
        .size(14)
        .padding(4);

    let header = row![
        text("Channel").size(13).width(Fill),
        text("Users").size(13),
        text("Topic").size(13).width(Fill),
    ]
    .spacing(8)
    .padding(4);

    let filter_lower = filter.to_lowercase();
    let mut list_col = column![].spacing(1);

    for entry in entries {
        let name = String::from_utf8_lossy(&entry.channel);
        if !filter_lower.is_empty() && !name.to_lowercase().contains(&filter_lower) {
            continue;
        }

        let chan_button = button(text(name.to_string()).size(12))
            .on_press(Msg::ListJoinChannel(entry.channel.clone()))
            .padding(2);

        let row = row![
            chan_button.width(Fill),
            text(entry.user_count.to_string()).size(12),
            text(&entry.topic).size(12).width(Fill),
        ]
        .spacing(8)
        .padding(2);

        list_col = list_col.push(row);
    }

    let status = if loading {
        text("Loading channel list…").size(11)
    } else {
        text(format!("{} channels", entries.len())).size(11)
    };

    let content = column![
        filter_input,
        header,
        scrollable(list_col).height(Fill),
        status
    ]
    .spacing(4);

    container(content)
        .width(Fill)
        .height(Fill)
        .padding(8)
        .into()
}
