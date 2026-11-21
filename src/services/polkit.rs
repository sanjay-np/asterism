//! The polkit authentication agent: when an app needs an administrator's say ("Authentication
//! is required to …"), polkit asks the agent of your session for a password. The shell is that
//! agent, unless another one already holds the session.
//!
//! The password is checked by polkit's own helper, never by the shell: through its socket on
//! recent polkit (the helper runs sandboxed per connection), or the setuid helper on older ones.
//! Both speak the same lines: the helper prompts (`PAM_PROMPT_ECHO_OFF Password: `), the agent
//! answers, the helper ends with `SUCCESS` or `FAILURE`.

use super::Out;
use super::dbus;
use crate::model::{AuthRequest, Command, Secret, Update};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";
const AGENT_PATH: &str = "/org/asterism/PolicyKit1/AuthenticationAgent";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPERS: [&str; 2] = ["/usr/lib/polkit-1/polkit-agent-helper-1", "/usr/libexec/polkit-agent-helper-1"];

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
    Failed(String),
}

enum Reply {
    Password(Secret),
    Cancel,
}

/// The requests polkit is waiting on, the one shown first.
#[derive(Default)]
struct Queue {
    requests: VecDeque<AuthRequest>,
    replies: HashMap<String, UnboundedSender<Reply>>,
}

struct Agent {
    queue: Arc<Mutex<Queue>>,
    out: Out,
}

fn publish(out: &Out, queue: &Queue) {
    let shown = queue.requests.front().cloned().map(|mut request| {
        request.waiting = queue.requests.len() - 1;
        request
    });
    out.send(Update::Auth(shown));
}

/// Users from /etc/passwd: uid → name.
fn users() -> HashMap<u32, String> {
    std::fs::read_to_string("/etc/passwd").map(|text| parse_passwd(&text)).unwrap_or_default()
}

fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

/// Members of a group from /etc/group, by gid.
fn group_members(gid: u32) -> Vec<String> {
    let text = std::fs::read_to_string("/etc/group").unwrap_or_default();
    text.lines()
        .find(|line| line.split(':').nth(2).and_then(|g| g.parse::<u32>().ok()) == Some(gid))
        .and_then(|line| line.split(':').nth(3))
        .map(|members| members.split(',').filter(|m| !m.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Who should type their password: you, when you're one of those allowed, else the first one.
fn pick_user(identities: &[(String, HashMap<String, OwnedValue>)], me: u32, users: &HashMap<u32, String>) -> Option<String> {
    let mut candidates = Vec::new();
    for (kind, details) in identities {
        let number = |key: &str| details.get(key).and_then(|v| dbus::as_f64(v)).map(|n| n as u32);
        match kind.as_str() {
            "unix-user" => {
                if let Some(name) = number("uid").and_then(|uid| users.get(&uid)) {
                    candidates.push(name.clone());
                }
            }
            "unix-group" => {
                if let Some(gid) = number("gid") {
                    candidates.extend(group_members(gid));
                }
            }
            _ => {}
        }
    }
    let mine = users.get(&me);
    match mine {
        Some(name) if candidates.contains(name) => Some(name.clone()),
        _ => candidates.into_iter().next(),
    }
}

/// How the helper's conversation ended.
#[derive(Debug, PartialEq)]
enum Outcome {
    Success,
    /// It didn't work; what PAM said about it, if anything.
    Failure(String),
}

/// Talks to the helper: answers its prompts and waits for its verdict.
async fn converse<R, W>(reader: R, mut writer: W, user: &str, password: &Secret) -> std::io::Result<Outcome>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut said = String::new();
    let mut answered = false;
    while let Some(line) = lines.next_line().await? {
        let (kind, text) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match kind {
            "PAM_PROMPT_ECHO_OFF" => {
                // One password per attempt: a second prompt means PAM wants something else
                let mut answer = if answered { Vec::new() } else { password.bytes().to_vec() };
                answer.push(b'\n');
                let written = writer.write_all(&answer).await;
                crate::model::wipe(&mut answer);
                written?;
                writer.flush().await?;
                answered = true;
            }
            "PAM_PROMPT_ECHO_ON" => {
                writer.write_all(format!("{user}\n").as_bytes()).await?;
                writer.flush().await?;
            }
            "PAM_ERROR_MSG" | "PAM_TEXT_INFO" => said = text.trim().to_string(),
            "SUCCESS" => return Ok(Outcome::Success),
            "FAILURE" => return Ok(Outcome::Failure(said)),
            _ => {}
        }
    }
    Ok(Outcome::Failure(said))
}

/// Checks a password through polkit's helper.
async fn check(user: &str, cookie: &str, password: &Secret) -> Outcome {
    let attempt = async {
        if let Ok(stream) = tokio::net::UnixStream::connect(HELPER_SOCKET).await {
            // Recent polkit: the helper takes the user and the cookie, one per line
            let (read, mut write) = stream.into_split();
            write.write_all(format!("{user}\n{cookie}\n").as_bytes()).await?;
            return converse(read, write, user, password).await;
        }
        let helper = HELPERS.iter().find(|h| std::path::Path::new(h).exists()).ok_or_else(|| std::io::Error::other("no polkit helper"))?;
        let mut child = super::child(helper).arg(user).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn()?;
        let mut stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        stdin.write_all(format!("{cookie}\n").as_bytes()).await?;
        let outcome = converse(stdout, stdin, user, password).await;
        let _ = child.wait().await;
        outcome
    };
    match tokio::time::timeout(Duration::from_secs(60), attempt).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            log::warn!("polkit helper: {error}");
            Outcome::Failure("The password couldn't be checked".into())
        }
        Err(_) => Outcome::Failure("Checking took too long".into()),
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        let me = rustix::process::getuid().as_raw();
        let Some(user) = pick_user(&identities, me, &users()) else {
            return Err(AgentError::Failed("No user to authenticate as".into()));
        };
        let (sender, mut replies) = unbounded_channel();
        {
            let mut queue = self.queue.lock().unwrap();
            queue.replies.insert(cookie.clone(), sender);
            queue.requests.push_back(AuthRequest { cookie: cookie.clone(), message, action: action_id, user: user.clone(), ..Default::default() });
            publish(&self.out, &queue);
        }
        let set = |change: &dyn Fn(&mut AuthRequest)| {
            let mut queue = self.queue.lock().unwrap();
            if let Some(request) = queue.requests.iter_mut().find(|r| r.cookie == cookie) {
                change(request);
            }
            publish(&self.out, &queue);
        };
        let finish = || {
            let mut queue = self.queue.lock().unwrap();
            queue.requests.retain(|r| r.cookie != cookie);
            queue.replies.remove(&cookie);
            publish(&self.out, &queue);
        };
        loop {
            match replies.recv().await {
                Some(Reply::Password(password)) => {
                    set(&|r| {
                        r.busy = true;
                        r.error.clear();
                    });
                    match check(&user, &cookie, &password).await {
                        Outcome::Success => {
                            finish();
                            return Ok(());
                        }
                        Outcome::Failure(said) => {
                            let error = if said.is_empty() || said.to_lowercase().contains("authentication failure") {
                                "That wasn't it. Try again.".to_string()
                            } else {
                                said
                            };
                            set(&|r| {
                                r.busy = false;
                                r.error = error.clone();
                            });
                        }
                    }
                }
                Some(Reply::Cancel) | None => {
                    finish();
                    return Err(AgentError::Cancelled("Cancelled".into()));
                }
            }
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        if let Some(sender) = self.queue.lock().unwrap().replies.get(&cookie) {
            let _ = sender.send(Reply::Cancel);
        }
    }
}

/// The session the agent speaks for: the one this runs in, else your graphical one.
pub(super) async fn session_id(bus: &Connection) -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID")
        && !id.is_empty()
    {
        return Some(id);
    }
    // A user service isn't in a session: logind knows which one is your display's
    if let Some(display) = dbus::get(bus, "org.freedesktop.login1", "/org/freedesktop/login1/user/self", "org.freedesktop.login1.User", "Display").await
        && let Value::Structure(s) = dbus::inner(&display)
        && let Some(id) = s.fields().first().and_then(|v| dbus::as_str(v))
        && !id.is_empty()
    {
        return Some(id);
    }
    dbus::get(bus, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "Id").await.and_then(|v| dbus::as_str(&v))
}

pub async fn run(out: Out, bus: Connection, mut commands: UnboundedReceiver<Command>) {
    let queue = Arc::new(Mutex::new(Queue::default()));
    let registered = async {
        bus.object_server().at(AGENT_PATH, Agent { queue: queue.clone(), out: out.clone() }).await.ok()?;
        let session = session_id(&bus).await?;
        let mut details: HashMap<&str, Value> = HashMap::new();
        details.insert("session-id", Value::from(session.clone()));
        let subject = ("unix-session", details);
        let locale = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into());
        match bus.call_method(Some(AUTHORITY), AUTHORITY_PATH, Some(AUTHORITY_INTERFACE), "RegisterAuthenticationAgent", &(subject, locale.as_str(), AGENT_PATH)).await {
            Ok(_) => {
                log::info!("polkit: the shell is the authentication agent for session {session}");
                Some(())
            }
            Err(error) => {
                log::info!("polkit: not the authentication agent ({error})");
                None
            }
        }
    };
    if registered.await.is_none() {
        let _ = bus.object_server().remove::<Agent, _>(AGENT_PATH).await;
    }
    while let Some(command) = commands.recv().await {
        let reply = match command {
            Command::AuthRespond(cookie, password) => Some((cookie, Reply::Password(password))),
            Command::AuthCancel(cookie) => Some((cookie, Reply::Cancel)),
            _ => None,
        };
        if let Some((cookie, reply)) = reply
            && let Some(sender) = queue.lock().unwrap().replies.get(&cookie)
        {
            let _ = sender.send(reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> Secret {
        let mut s = Secret::default();
        s.push(text);
        s
    }

    /// A pretend helper: prompts, reads the answer, and says whether it was right.
    async fn helper(script: Vec<&'static str>, expected: &'static str) -> Outcome {
        let (agent_side, helper_side) = tokio::io::duplex(1024);
        let (agent_read, agent_write) = tokio::io::split(agent_side);
        let (helper_read, mut helper_write) = tokio::io::split(helper_side);
        let pretend = tokio::spawn(async move {
            let mut lines = BufReader::new(helper_read).lines();
            for line in script {
                helper_write.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                if line.starts_with("PAM_PROMPT") {
                    let answer = lines.next_line().await.unwrap().unwrap();
                    let verdict = if answer == expected { "SUCCESS" } else { "FAILURE" };
                    helper_write.write_all(format!("{verdict}\n").as_bytes()).await.unwrap();
                    return;
                }
            }
        });
        let outcome = converse(agent_read, agent_write, "maya", &secret("hunter2")).await.unwrap();
        pretend.await.unwrap();
        outcome
    }

    #[tokio::test]
    async fn answers_the_helper() {
        assert_eq!(helper(vec!["PAM_TEXT_INFO hello", "PAM_PROMPT_ECHO_OFF Password: "], "hunter2").await, Outcome::Success);
        assert_eq!(helper(vec!["PAM_PROMPT_ECHO_OFF Password: "], "something else").await, Outcome::Failure(String::new()));
    }

    #[test]
    fn picks_who_types_the_password() {
        let users = parse_passwd("root:x:0:0::/root:/bin/bash\nmaya:x:1000:1000::/home/maya:/bin/zsh\nsam:x:1001:1001::/home/sam:/bin/zsh\n");
        let user = |uid: u32| ("unix-user".to_string(), HashMap::from([("uid".to_string(), OwnedValue::from(uid))]));
        assert_eq!(pick_user(&[user(0), user(1000)], 1000, &users), Some("maya".into()));
        assert_eq!(pick_user(&[user(0), user(1001)], 1000, &users), Some("root".into()));
        assert_eq!(pick_user(&[], 1000, &users), None);
    }

    #[test]
    fn secrets_forget() {
        let mut s = secret("pä");
        s.pop();
        assert_eq!(s.bytes(), b"p");
        assert_eq!(s.len(), 1);
        let taken = s.take();
        assert!(s.is_empty());
        assert_eq!(taken.bytes(), b"p");
        assert_eq!(format!("{taken:?}"), "Secret(…)");
    }
}
