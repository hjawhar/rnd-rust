use iced::widget::{column, container, scrollable, text};
use iced::{Element, Fill, Length};

use crate::app::Msg;

/// Render the right-hand nick list.
pub(crate) fn view(nicks: &[String]) -> Element<'_, Msg> {
    let mut col = column![].spacing(1);

    for nick in nicks {
        col = col.push(text(nick).size(12));
    }

    container(scrollable(col).height(Fill))
        .style(crate::theme::sidebar)
        .width(Length::Fixed(140.0))
        .height(Fill)
        .into()
}
