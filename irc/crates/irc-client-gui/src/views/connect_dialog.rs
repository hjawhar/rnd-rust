use iced::widget::{button, checkbox, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill, Length};

use crate::app::Msg;

/// Form state for the connect dialog.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConnectForm {
    pub host: String,
    pub port: String,
    pub nick: String,
    pub user: String,
    pub realname: String,
    pub tls: bool,
}

impl ConnectForm {
    pub(crate) fn default_local() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: "6667".into(),
            nick: "user".into(),
            user: "user".into(),
            realname: "IRC User".into(),
            tls: false,
        }
    }
}

pub(crate) fn view(form: &ConnectForm) -> Element<'_, Msg> {
    let title = text("Connect to Server").size(20);

    let host_input = column![
        text("Server address:").size(13),
        text_input("irc.example.net", &form.host)
            .on_input(|v| Msg::ConnectFormChanged(ConnectField::Host(v)))
            .padding(6)
    ]
    .spacing(4);

    let port_input = column![
        text("Port:").size(13),
        text_input("6667", &form.port)
            .on_input(|v| Msg::ConnectFormChanged(ConnectField::Port(v)))
            .padding(6)
            .width(80)
    ]
    .spacing(4);

    let nick_input = column![
        text("Nickname:").size(13),
        text_input("nickname", &form.nick)
            .on_input(|v| Msg::ConnectFormChanged(ConnectField::Nick(v)))
            .padding(6)
    ]
    .spacing(4);

    let user_input = column![
        text("Username:").size(13),
        text_input("username", &form.user)
            .on_input(|v| Msg::ConnectFormChanged(ConnectField::User(v)))
            .padding(6)
    ]
    .spacing(4);

    let realname_input = column![
        text("Real name:").size(13),
        text_input("Real Name", &form.realname)
            .on_input(|v| Msg::ConnectFormChanged(ConnectField::Realname(v)))
            .padding(6)
    ]
    .spacing(4);

    let tls_check = checkbox("Use TLS (SSL)", form.tls)
        .on_toggle(|v| Msg::ConnectFormChanged(ConnectField::Tls(v)));

    let connect_btn = button(text("Connect").size(14))
        .padding([6, 20])
        .on_press(Msg::ConnectSubmit);

    let cancel_btn = button(text("Cancel").size(14))
        .padding([6, 20])
        .on_press(Msg::ConnectCancel);

    let buttons = row![cancel_btn, connect_btn]
        .spacing(10)
        .align_y(Alignment::Center);

    let form_col = column![
        title,
        host_input,
        row![port_input, tls_check]
            .spacing(20)
            .align_y(Alignment::End),
        nick_input,
        user_input,
        realname_input,
        buttons,
    ]
    .spacing(12)
    .width(360)
    .padding(20);

    container(form_col)
        .width(Fill)
        .height(Fill)
        .center(Length::Fill)
        .into()
}

/// Which field changed in the connect form.
#[derive(Debug, Clone)]
pub(crate) enum ConnectField {
    Host(String),
    Port(String),
    Nick(String),
    User(String),
    Realname(String),
    Tls(bool),
}
