use iced::widget::{container, text_input};
use iced::{Element, Fill};

use crate::app::Msg;

/// Render the bottom input bar.
pub(crate) fn view(value: &str) -> Element<'_, Msg> {
    container(
        text_input("Type a message…", value)
            .on_input(Msg::InputChanged)
            .on_submit(Msg::InputSubmit)
            .size(13)
            .width(Fill),
    )
    .width(Fill)
    .padding(4)
    .into()
}
