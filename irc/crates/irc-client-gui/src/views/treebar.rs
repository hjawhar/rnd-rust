use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Fill, Length};

use irc_client_core::NetworkId;

use crate::app::{Msg, NetworkInfo, WindowId};

/// Render the mIRC-style server/channel tree.
///
/// Shows each connected network as a bold header with its status window,
/// followed by indented channel/query entries.
pub(crate) fn view<'a>(
    networks: impl Iterator<Item = (&'a NetworkId, &'a NetworkInfo)>,
    active: Option<WindowId>,
) -> Element<'a, Msg> {
    let mut items: Vec<Element<'a, Msg>> = Vec::new();

    // "New Connection" button at the top.
    items.push(
        button(text("+ Connect").size(12))
            .padding([4, 8])
            .on_press(Msg::ShowConnectDialog)
            .into(),
    );

    // Sort networks by name for stable ordering.
    let mut nets: Vec<_> = networks.collect();
    nets.sort_by(|a, b| a.1.name.cmp(&b.1.name));

    for (_net_id, info) in nets {
        // Network header — clicking goes to the status window.
        let conn_indicator = if info.connected { "●" } else { "○" };
        let header_text = format!("{conn_indicator} {}", info.name);
        let is_active = active == Some(info.status_window);
        let header = button(text(header_text).size(13).color(if is_active {
            iced::Color::from_rgb(0.4, 0.8, 1.0)
        } else {
            iced::Color::from_rgb(0.8, 0.8, 0.8)
        }))
        .padding([3, 6])
        .width(Fill)
        .on_press(Msg::WindowSelected(info.status_window));

        items.push(header.into());

        // Channel/query windows under this network.
        for wref in &info.windows {
            let target = String::from_utf8_lossy(&wref.target);
            let is_chan_active = active == Some(wref.id);
            let label = format!("  {target}");
            let entry = button(text(label).size(12).color(if is_chan_active {
                iced::Color::from_rgb(0.4, 0.8, 1.0)
            } else if target.starts_with('#') {
                iced::Color::from_rgb(0.7, 0.7, 0.7)
            } else {
                iced::Color::from_rgb(0.9, 0.7, 0.5) // queries in warm color
            }))
            .padding([2, 6])
            .width(Fill)
            .on_press(Msg::WindowSelected(wref.id));

            items.push(entry.into());
        }
    }

    let tree = column(items).spacing(1).width(180);

    container(scrollable(tree).height(Fill))
        .width(Length::Fixed(180.0))
        .into()
}
