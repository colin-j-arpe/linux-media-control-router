use super::Result;
use crate::player::routing::TransportAction;
use std::{
    collections::{BTreeMap, HashSet},
    os::fd::{AsRawFd, RawFd},
};
use tokio::io::unix::AsyncFd;
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xkb::{self, ConnectionExt as _},
        xproto::{ConnectionExt, GrabMode, Mapping, ModMask},
    },
    rust_connection::RustConnection,
};

struct Descriptor(RawFd);
impl AsRawFd for Descriptor {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

pub(crate) struct Keyboard {
    // Deregister readiness before dropping its underlying connection.
    ready: AsyncFd<Descriptor>,
    connection: RustConnection,
    keys: BTreeMap<u8, TransportAction>,
    modifiers: Vec<u16>,
    pressed: HashSet<u8>,
}
impl Keyboard {
    pub fn connect() -> Result<Self> {
        let (connection, _) = x11rb::connect(None)?;
        if !connection.xkb_use_extension(1, 0)?.reply()?.supported {
            return Err("XKB extension is required".into());
        }
        let flags = connection
            .xkb_per_client_flags(
                xkb::ID::USE_CORE_KBD.into(),
                xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
                xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT,
                0u32.into(),
                0u32.into(),
                0u32.into(),
            )?
            .reply()?;
        if !flags
            .value
            .contains(xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT)
        {
            return Err("X server does not support detectable key repeat".into());
        }
        let map_parts = xkb::MapPart::KEY_SYMS | xkb::MapPart::MODIFIER_MAP;
        connection
            .xkb_select_events(
                xkb::ID::USE_CORE_KBD.into(),
                0u16.into(),
                xkb::EventType::NEW_KEYBOARD_NOTIFY | xkb::EventType::MAP_NOTIFY,
                map_parts,
                map_parts,
                &xkb::SelectEventsAux::default(),
            )?
            .check()?;
        let (keys, modifiers) = key_map(&connection)?;
        let ready = AsyncFd::new(Descriptor(connection.stream().as_raw_fd()))?;
        Ok(Self {
            ready,
            connection,
            keys,
            modifiers,
            pressed: HashSet::new(),
        })
    }
    pub fn grab(&self) -> Result<()> {
        for screen in &self.connection.setup().roots {
            for &code in self.keys.keys() {
                for &modifier in &self.modifiers {
                    if let Err(error) = self
                        .connection
                        .grab_key(
                            false,
                            screen.root,
                            ModMask::from(modifier),
                            code,
                            GrabMode::ASYNC,
                            GrabMode::ASYNC,
                        )?
                        .check()
                    {
                        self.ungrab()?;
                        return Err(error.into());
                    }
                }
            }
        }
        self.connection.flush()?;
        Ok(())
    }
    fn ungrab(&self) -> Result<()> {
        for screen in &self.connection.setup().roots {
            for &code in self.keys.keys() {
                for &modifier in &self.modifiers {
                    self.connection
                        .ungrab_key(code, screen.root, ModMask::from(modifier))?
                        .check()?;
                }
            }
        }
        Ok(())
    }
    fn check_mapping(&self) -> Result<()> {
        let (keys, modifiers) = key_map(&self.connection)?;
        if keys != self.keys || modifiers != self.modifiers {
            return Err("transport keyboard mapping changed; capture stopped, restart to use the new mapping".into());
        }
        Ok(())
    }
    pub async fn next(&mut self) -> Result<TransportAction> {
        loop {
            if let Some(event) = self.connection.poll_for_event()? {
                match event {
                    Event::KeyPress(event) if event.response_type & 0x80 == 0 => {
                        if self.modifiers.contains(&(u16::from(event.state) & 0xff))
                            && self.pressed.insert(event.detail)
                            && let Some(action) = self.keys.get(&event.detail)
                        {
                            return Ok(*action);
                        }
                    }
                    Event::KeyRelease(event) if event.response_type & 0x80 == 0 => {
                        self.pressed.remove(&event.detail);
                    }
                    Event::MappingNotify(event) if event.request != Mapping::POINTER => {
                        self.check_mapping()?;
                    }
                    Event::XkbNewKeyboardNotify(_) | Event::XkbMapNotify(_) => {
                        self.check_mapping()?;
                    }
                    Event::Error(error) => return Err(format!("X11 event error: {error:?}").into()),
                    _ => (),
                }
                continue;
            }
            let mut readiness = self.ready.readable().await?;
            // x11rb owns the nonblocking read and may already have buffered
            // events. Always poll its queue before waiting for another edge.
            readiness.clear_ready();
        }
    }
}
type KeyMap = (BTreeMap<u8, TransportAction>, Vec<u16>);
fn key_map(connection: &RustConnection) -> Result<KeyMap> {
    let setup = connection.setup();
    let mapping = connection
        .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
        .reply()?;
    let width = usize::from(mapping.keysyms_per_keycode);
    if width == 0 {
        return Err("empty X11 keyboard mapping".into());
    }
    let mut keys = BTreeMap::new();
    let mut locks = HashSet::new();
    for (offset, symbols) in mapping.keysyms.chunks_exact(width).enumerate() {
        let code = setup.min_keycode + offset as u8;
        // Only the base symbol: shifted/alternate levels are not hardware
        // transport keys and must not cause grabs of ordinary letter keys.
        if let Some(transport) = action(symbols[0]) {
            keys.insert(code, transport);
        }
        if symbols
            .iter()
            .any(|s| matches!(s, 0xffe5 | 0xffe6 | 0xff7f | 0xff14))
        {
            locks.insert(code);
        }
    }
    if keys.is_empty() {
        return Err("no standard transport keys in the X11 keyboard mapping".into());
    }
    let mapping = connection.get_modifier_mapping()?.reply()?;
    let mut mask = 0u16;
    let count = usize::from(mapping.keycodes_per_modifier());
    if count > 0 {
        for (index, codes) in mapping.keycodes.chunks_exact(count).enumerate() {
            if codes.iter().any(|code| locks.contains(code)) {
                if codes.iter().any(|code| *code != 0 && !locks.contains(code)) {
                    return Err("lock and shortcut modifiers share an X11 modifier slot; cannot safely capture plain media keys".into());
                }
                mask |= 1 << index;
            }
        }
    }
    let modifiers = (0..256u16).filter(|value| value & !mask == 0).collect();
    Ok((keys, modifiers))
}

fn action(symbol: u32) -> Option<TransportAction> {
    match symbol {
        0x1008ff14 | 0x1008ff31 => Some(TransportAction::PlayPause),
        0x1008ff15 => Some(TransportAction::Stop),
        0x1008ff16 => Some(TransportAction::Previous),
        0x1008ff17 => Some(TransportAction::Next),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_transport_only() {
        assert_eq!(action(0x1008ff14), action(0x1008ff31));
        for symbol in [0x1008ff11, 0x1008ff12, 0x1008ff13, 0x61] {
            assert_eq!(action(symbol), None);
        }
        assert_eq!(action(0x1008ff15), Some(TransportAction::Stop));
        assert_eq!(action(0x1008ff16), Some(TransportAction::Previous));
        assert_eq!(action(0x1008ff17), Some(TransportAction::Next));
    }
}
