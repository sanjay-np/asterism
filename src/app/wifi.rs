//! The wifi password prompt: a secured network you haven't joined asks for its password on a
//! card like polkit's (a dot a character); NetworkManager is given it directly and keeps it
//! only if it works.

use super::*;
use crate::model::Secret;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

pub(super) struct WifiAsk {
    pub ssid: String,
    pub password: Secret,
    pub error: String,
    pub busy: bool,
}

impl App {
    pub(super) fn wifi_ask(&mut self, ssid: String) {
        self.close_menu();
        self.wifi = Some(WifiAsk { ssid, password: Secret::default(), error: String::new(), busy: false });
        if self.panel_index(PanelKind::Wifi).is_none() {
            self.open_panel(PanelKind::Wifi, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
        }
    }

    pub(super) fn wifi_pieces(&mut self, w: f32, h: f32) -> Option<Vec<Piece>> {
        let ask = self.wifi.as_ref()?;
        let title = format!("Connect to “{}”", ask.ssid);
        let card = crate::cards::Ask {
            key: "wifi",
            icon: "wifi-high",
            title: &title,
            message: "This network is secured. Type its password to join it.",
            note: "Kept by NetworkManager once it works",
            busy: ask.busy,
            error: &ask.error,
            ok: "Connect",
            ok_action: Action::WifiSubmit,
            cancel_action: Action::WifiCancel,
        };
        Some(crate::cards::password_card(&self.theme, &mut self.painter.text, &card, ask.password.len(), w, h))
    }

    pub(super) fn wifi_submit(&mut self) {
        let Some(ask) = self.wifi.as_mut() else { return };
        if ask.busy || ask.password.is_empty() {
            return;
        }
        ask.busy = true;
        ask.error.clear();
        let command = Command::ConnectWifiWith(ask.ssid.clone(), ask.password.take());
        self.send(command);
        self.wifi_dirty();
    }

    pub(super) fn wifi_cancel(&mut self) {
        self.wifi = None;
        self.close_panel(PanelKind::Wifi);
    }

    /// NetworkManager's answer: joined (the card goes, a word says so), or not (it says why).
    pub(super) fn wifi_joined(&mut self, ssid: String, ok: bool, why: String) {
        if ok {
            if self.wifi.as_ref().is_some_and(|a| a.ssid == ssid) {
                self.wifi_cancel();
            }
            self.send(Command::Notify(crate::model::Note { summary: format!("Connected to {ssid}"), icon: "network-wireless".into(), ..Default::default() }));
            return;
        }
        if let Some(ask) = self.wifi.as_mut().filter(|a| a.ssid == ssid) {
            ask.busy = false;
            ask.error = why;
            ask.password.clear();
            self.wifi_dirty();
        }
    }

    fn wifi_dirty(&mut self) {
        if let Some(index) = self.panel_index(PanelKind::Wifi) {
            self.panels[index].surf.dirty = true;
        }
    }

    pub(super) fn wifi_key(&mut self, event: &KeyEvent) {
        let (ctrl, alt) = (self.modifiers.ctrl, self.modifiers.alt);
        let Some(ask) = self.wifi.as_mut() else { return };
        match event.keysym {
            Keysym::Escape => return self.wifi_cancel(),
            Keysym::Return | Keysym::KP_Enter => return self.wifi_submit(),
            _ if ask.busy => return,
            Keysym::BackSpace if ctrl => ask.password.clear(),
            Keysym::BackSpace => ask.password.pop(),
            Keysym::u if ctrl => ask.password.clear(),
            _ => match &event.utf8 {
                Some(text) if !ctrl && !alt && !text.chars().any(char::is_control) => ask.password.push(text),
                _ => return,
            },
        }
        ask.error.clear();
        self.wifi_dirty();
    }
}

// ---------------------------------------------------------------- bluetooth pairing

impl App {
    /// BlueZ asks (or stops asking): the card follows.
    pub(super) fn pairing(&mut self, pairing: Option<crate::model::Pairing>) {
        match pairing {
            Some(pairing) => {
                self.close_menu();
                self.pair = Some((pairing, String::new()));
                if self.panel_index(PanelKind::Pair).is_none() {
                    self.open_panel(PanelKind::Pair, Layer::Overlay, Anchor::all(), (0, 0), (0, 0, 0, 0), KeyboardInteractivity::Exclusive, -1);
                } else if let Some(index) = self.panel_index(PanelKind::Pair) {
                    self.panels[index].surf.dirty = true;
                }
            }
            None => {
                self.pair = None;
                self.close_panel(PanelKind::Pair);
            }
        }
    }

    /// Yes (with what was typed) or no; the card goes either way.
    pub(super) fn pair_answer(&mut self, answer: Option<String>) {
        self.send(Command::PairAnswer(answer));
        self.pair = None;
        self.close_panel(PanelKind::Pair);
    }

    pub(super) fn pair_key(&mut self, event: &KeyEvent) {
        use crate::model::PairAsk;
        let Some((pairing, typed)) = self.pair.as_mut() else { return };
        let digits_only = pairing.ask == PairAsk::Passkey;
        let typing = matches!(pairing.ask, PairAsk::Pin | PairAsk::Passkey);
        match event.keysym {
            Keysym::Escape => return self.pair_answer(None),
            Keysym::Return | Keysym::KP_Enter => {
                let answer = match &pairing.ask {
                    PairAsk::ShowPin(_) | PairAsk::ShowPasskey(_) => None,
                    _ if typing && typed.is_empty() => return,
                    _ if typing => Some(typed.clone()),
                    _ => Some(String::new()),
                };
                return self.pair_answer(answer);
            }
            Keysym::BackSpace if typing => {
                typed.pop();
            }
            _ if typing => match &event.utf8 {
                Some(text) if !self.modifiers.ctrl && text.chars().all(|c| if digits_only { c.is_ascii_digit() } else { !c.is_control() }) && typed.len() < 16 => typed.push_str(text),
                _ => return,
            },
            _ => return,
        }
        if let Some(index) = self.panel_index(PanelKind::Pair) {
            self.panels[index].surf.dirty = true;
        }
    }
}
