use eframe::egui::{Context, TextEdit, Ui};
use reqwest::{Method, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Copy, Default, PartialEq)]
enum Action {
    #[default]
    Login,
    Register,
    Verify,
    VerifyLogin,
    Forgot,
    Reset,
    Logout,
    Download,
    Upload,
    Session,
}

impl Action {
    fn path(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Register => "register",
            Self::Verify => "verify-email",
            Self::VerifyLogin => "verify-login",
            Self::Forgot => "forgot-password",
            Self::Reset => "reset-password",
            Self::Logout => "logout",
            Self::Download | Self::Upload => "data",
            Self::Session => "me",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Login => "Sign in",
            Self::Register | Self::Forgot => "Send email code",
            Self::Verify | Self::VerifyLogin | Self::Reset => "Confirm",
            Self::Logout => "Sign out",
            Self::Download => "Restore from account",
            Self::Upload => "Save to account",
            Self::Session => "Check session",
        }
    }
}

#[derive(Default)]
pub struct Account {
    email: String,
    password: String,
    code: String,
    action: Action,
    token: Option<String>,
    verified: bool,
    recheck: Option<std::time::Instant>,
    revision: u64,
    restored: Option<Value>,
    pending: Option<mpsc::Receiver<(Action, Result<Value, String>)>>,
    notice: String,
}

impl Account {
    pub fn signed_in(&mut self, ctx: &Context) -> bool {
        self.receive();
        if self.verified && self.token.is_some() && self.pending.is_none() {
            let now = std::time::Instant::now();
            if let Some(at) = self.recheck.filter(|at| *at > now) {
                ctx.request_repaint_after(at.duration_since(now));
            } else {
                self.recheck = Some(now + Duration::from_secs(60));
                self.send(Action::Session, Value::Null, ctx);
            }
        }
        self.verified && self.token.is_some()
    }

    pub fn show(&mut self, ui: &mut Ui, data: impl FnOnce() -> Value) -> Option<Value> {
        let busy = self.pending.is_some();
        ui.label(crate::style::label("Vortex account", 21.0));
        if self.token.is_some() {
            ui.label(&self.email);
            if self.verified {
                ui.colored_label(eframe::egui::Color32::LIGHT_GREEN, "Email verified");
            }
            ui.label("Save or restore your library, watch history, search history, and desktop preferences.");
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(Action::Download.caption()).clicked() {
                        self.submit(Action::Download, ui.ctx());
                    }
                    if ui.button(Action::Upload.caption()).clicked() {
                        self.send(
                            Action::Upload,
                            json!({"revision": self.revision, "data": data()}),
                            ui.ctx(),
                        );
                    }
                    if ui.button("Sign out").clicked() {
                        self.submit(Action::Logout, ui.ctx());
                    }
                });
            });
        } else {
            ui.add_enabled_ui(!busy, |ui| {
                ui.label(match self.action {
                    Action::Register => "Create an account",
                    Action::Verify => "Verify your email and choose a password",
                    Action::VerifyLogin => "Enter the one-time code sent to your Gmail inbox",
                    Action::Forgot => "Request a password reset",
                    Action::Reset => "Use the email code to choose a new password",
                    _ => "Sign in with your Vortex account",
                });
                ui.add(TextEdit::singleline(&mut self.email).hint_text("Email"));
                if matches!(
                    self.action,
                    Action::Verify | Action::VerifyLogin | Action::Reset
                ) {
                    ui.add(TextEdit::singleline(&mut self.code).hint_text("Code from your email"));
                }
                if matches!(self.action, Action::Login | Action::Verify | Action::Reset) {
                    ui.add(
                        TextEdit::singleline(&mut self.password)
                            .hint_text("Password")
                            .password(true),
                    );
                }
                if ui.button(self.action.caption()).clicked() {
                    self.submit(self.action, ui.ctx());
                }
                ui.horizontal_wrapped(|ui| {
                    if self.action == Action::Login {
                        if ui.button("Create account").clicked() {
                            self.select(Action::Register);
                        }
                        if ui.button("Forgot password").clicked() {
                            self.select(Action::Forgot);
                        }
                    } else if ui.button("Back to sign in").clicked() {
                        self.select(Action::Login);
                    }
                });
            });
        }
        if busy {
            ui.spinner();
        }
        if !self.notice.is_empty() {
            ui.label(&self.notice);
        }
        self.restored.take()
    }

    fn select(&mut self, action: Action) {
        self.action = action;
        self.password.clear();
        self.code.clear();
        self.notice.clear();
    }

    fn submit(&mut self, action: Action, ctx: &Context) {
        self.notice.clear();
        let body = json!({
            "email": self.email.trim(),
            "password": std::mem::take(&mut self.password),
            "code": self.code.trim(),
        });
        self.send(action, body, ctx);
    }

    fn send(&mut self, action: Action, body: Value, ctx: &Context) {
        if action != Action::Session {
            self.notice.clear();
        }
        let token = self.token.clone();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = request(action, token.as_deref(), &body);
            let _ = sender.send((action, result));
            ctx.request_repaint();
        });
    }

    fn receive(&mut self) {
        let Some(reply) = self.pending.as_ref().map(mpsc::Receiver::try_recv) else {
            return;
        };
        let (action, result) = match reply {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.notice = "The account request was interrupted.".to_owned();
                return;
            }
        };
        self.pending = None;
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                if action == Action::Session {
                    return;
                }
                self.notice = error;
                return;
            }
        };
        match action {
            Action::Login if response["otpRequired"].as_bool() == Some(true) => {
                self.select(Action::VerifyLogin);
                self.notice = "Check your Gmail inbox for your sign-in code.".to_owned();
            }
            Action::Login | Action::VerifyLogin => {
                self.token = response["token"].as_str().map(str::to_owned);
                self.verified = response["emailVerified"].as_bool() == Some(true);
                self.recheck = Some(std::time::Instant::now() + Duration::from_secs(60));
                self.revision = 0;
                if self.token.is_none() || !self.verified {
                    self.token = None;
                    self.notice = "The account server returned an invalid session.".to_owned();
                }
            }
            Action::Logout => {
                self.token = None;
                self.verified = false;
                self.select(Action::Login);
            }
            Action::Session => {
                if response["emailVerified"].as_bool() != Some(true) {
                    self.token = None;
                    self.verified = false;
                    self.select(Action::Login);
                    self.notice = "Your session ended. Sign in to continue.".to_owned();
                }
            }
            Action::Download => {
                self.revision = response["revision"].as_u64().unwrap_or_default();
                self.restored = response.get("data").cloned();
                self.notice = "Account data restored.".to_owned();
            }
            Action::Upload => {
                self.revision = response["revision"].as_u64().unwrap_or_default();
                self.notice = "Data saved to your account.".to_owned();
            }
            Action::Verify | Action::Reset => {
                self.select(Action::Login);
                self.notice = "You can now sign in with your password.".to_owned();
            }
            Action::Register | Action::Forgot => {
                self.select(if action == Action::Register {
                    Action::Verify
                } else {
                    Action::Reset
                });
                self.notice = "If the email is eligible, you’ll receive a code.".to_owned();
            }
        }
    }
}

fn request(action: Action, token: Option<&str>, body: &Value) -> Result<Value, String> {
    let endpoint = std::env::var("VORTEX_ACCOUNT_API_URL")
        .ok()
        .or_else(|| option_env!("VORTEX_ACCOUNT_API_URL").map(str::to_owned));
    let endpoint = endpoint
        .filter(|url| url.starts_with("https://"))
        .ok_or_else(|| "This build has no configured Vortex account service.".to_owned())?;
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "Could not start the account connection.".to_owned())?;
    let method = match action {
        Action::Download | Action::Session => Method::GET,
        Action::Upload => Method::PATCH,
        _ => Method::POST,
    };
    let mut request = client
        .request(
            method,
            format!("{}/{}", endpoint.trim_end_matches('/'), action.path()),
        )
        .header("X-Vortex-Client", "desktop");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if !matches!(action, Action::Download | Action::Session) {
        request = request.json(body);
    }
    let response = request
        .send()
        .map_err(|_| "Could not reach the account service.".to_owned())?;
    let status = response.status();
    if status.as_u16() == 204
        || (matches!(action, Action::Logout | Action::Session) && status.as_u16() == 401)
    {
        return Ok(Value::Null);
    }
    if status.is_success() {
        return response
            .json()
            .map_err(|_| "Could not read the account response.".to_owned());
    }
    if status.as_u16() == 409 {
        return Err(
            "Data changed on another client. Restore from your account before saving again."
                .to_owned(),
        );
    }
    Err(response
        .json::<Value>()
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "The account service could not complete the request.".to_owned()))
}
