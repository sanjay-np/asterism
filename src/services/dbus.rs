//! Small helpers over zbus: read properties, call methods, and turn D-Bus values into plain
//! Rust ones without a proxy type for every interface.

use std::collections::HashMap;
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

pub type Props = HashMap<String, OwnedValue>;

pub async fn get_all(bus: &Connection, dest: &str, path: &str, interface: &str) -> Option<Props> {
    let reply = bus
        .call_method(Some(dest), path, Some("org.freedesktop.DBus.Properties"), "GetAll", &(interface,))
        .await
        .ok()?;
    reply.body().deserialize::<Props>().ok()
}

pub async fn get(bus: &Connection, dest: &str, path: &str, interface: &str, name: &str) -> Option<OwnedValue> {
    let reply = bus
        .call_method(Some(dest), path, Some("org.freedesktop.DBus.Properties"), "Get", &(interface, name))
        .await
        .ok()?;
    reply.body().deserialize::<OwnedValue>().ok()
}

pub async fn set<'a>(bus: &Connection, dest: &str, path: &str, interface: &str, name: &str, value: Value<'a>) {
    if let Err(error) = bus
        .call_method(Some(dest), path, Some("org.freedesktop.DBus.Properties"), "Set", &(interface, name, value))
        .await
    {
        log::warn!("setting {interface}.{name}: {error}");
    }
}

/// Calls a method and ignores the answer, apart from saying when it failed.
pub async fn call<B>(bus: &Connection, dest: &str, path: &str, interface: &str, method: &str, body: &B)
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    if let Err(error) = bus.call_method(Some(dest), path, Some(interface), method, body).await {
        log::debug!("{interface}.{method} on {dest}: {error}");
    }
}

/// Looks through variant wrappers to the value inside.
pub fn inner<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(boxed) => inner(boxed),
        other => other,
    }
}

pub fn as_str(value: &Value<'_>) -> Option<String> {
    match inner(value) {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        Value::Signature(s) => Some(s.to_string()),
        _ => None,
    }
}

pub fn as_f64(value: &Value<'_>) -> Option<f64> {
    Some(match inner(value) {
        Value::U8(v) => *v as f64,
        Value::I16(v) => *v as f64,
        Value::U16(v) => *v as f64,
        Value::I32(v) => *v as f64,
        Value::U32(v) => *v as f64,
        Value::I64(v) => *v as f64,
        Value::U64(v) => *v as f64,
        Value::F64(v) => *v,
        _ => return None,
    })
}

pub fn as_bool(value: &Value<'_>) -> Option<bool> {
    match inner(value) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// An array of strings (or object paths).
pub fn as_strings(value: &Value<'_>) -> Vec<String> {
    match inner(value) {
        Value::Array(array) => array.inner().iter().filter_map(as_str).collect(),
        other => as_str(other).into_iter().collect(),
    }
}

/// An array of bytes, like a wifi network's name.
pub fn as_bytes(value: &Value<'_>) -> Vec<u8> {
    match inner(value) {
        Value::Array(array) => array
            .inner()
            .iter()
            .filter_map(|v| match v {
                Value::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn text(props: &Props, key: &str) -> String {
    props.get(key).and_then(|v| as_str(v)).unwrap_or_default()
}

pub fn number(props: &Props, key: &str) -> f64 {
    props.get(key).and_then(|v| as_f64(v)).unwrap_or(0.0)
}

pub fn flag(props: &Props, key: &str) -> bool {
    props.get(key).and_then(|v| as_bool(v)).unwrap_or(false)
}

/// A dictionary of string keys, like `a{sv}`.
pub fn dict<'a>(value: &'a Value<'a>) -> HashMap<String, &'a Value<'a>> {
    let mut map = HashMap::new();
    if let Value::Dict(dict) = inner(value) {
        for (key, value) in dict.iter() {
            if let Some(key) = as_str(key) {
                map.insert(key, inner(value));
            }
        }
    }
    map
}

/// A stream of the signals matching a rule, or nothing if the bus refuses.
pub async fn signals(bus: &Connection, rule: zbus::MatchRule<'static>) -> Option<zbus::MessageStream> {
    match zbus::MessageStream::for_match_rule(rule, bus, Some(64)).await {
        Ok(stream) => Some(stream),
        Err(error) => {
            log::warn!("can't listen for signals: {error}");
            None
        }
    }
}

pub fn rule() -> zbus::match_rule::Builder<'static> {
    zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal)
}

/// Every signal matching any of the rules, in one channel.
pub async fn watch(bus: &Connection, rules: Vec<zbus::Result<zbus::MatchRule<'static>>>) -> tokio::sync::mpsc::UnboundedReceiver<zbus::Message> {
    use futures_lite::StreamExt;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    for rule in rules {
        let rule = match rule {
            Ok(rule) => rule,
            Err(error) => {
                log::warn!("bad match rule: {error}");
                continue;
            }
        };
        let Some(mut stream) = signals(bus, rule).await else { continue };
        let sender = sender.clone();
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                if let Ok(message) = message
                    && sender.send(message).is_err()
                {
                    break;
                }
            }
        });
    }
    receiver
}
