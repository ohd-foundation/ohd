//! Server-rendered HTML for the login + sign-up UI.
//!
//! Plain SSR — no SPA, no client JS. Every page is one self-contained
//! document with inline CSS, styled minimally around the OHD red accent
//! (`#E11D2A`). The authorize-flow parameters are threaded through every
//! form as hidden inputs so a `POST /login` or `POST /signup` can resume
//! the flow exactly where `/authorize` left off.

/// HTML-escape a string for safe interpolation into element text or a
/// double-quoted attribute value.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The shared `<head>` + inline stylesheet. OHD red accent.
const STYLE: &str = r#"<style>
  :root { --ohd-red: #E11D2A; }
  * { box-sizing: border-box; }
  body { margin: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI",
    Roboto, Helvetica, Arial, sans-serif; background: #f5f5f7; color: #1d1d1f;
    display: flex; min-height: 100vh; align-items: center; justify-content: center; }
  .card { background: #fff; border-radius: 14px; padding: 2.5rem 2.25rem;
    width: 100%; max-width: 380px; box-shadow: 0 2px 24px rgba(0,0,0,0.08); }
  .brand { font-weight: 700; font-size: 1.15rem; margin: 0 0 0.25rem; }
  .brand .accent { color: var(--ohd-red); }
  h1 { font-size: 1.35rem; margin: 0 0 1.5rem; }
  label { display: block; font-size: 0.85rem; font-weight: 600;
    margin: 1rem 0 0.35rem; }
  input[type=email], input[type=password], textarea { width: 100%;
    padding: 0.6rem 0.7rem; font-size: 1rem; border: 1px solid #d2d2d7;
    border-radius: 8px; font-family: inherit; }
  textarea { resize: vertical; font-family: ui-monospace, SFMono-Regular,
    Menlo, monospace; letter-spacing: 0.04em; }
  input:focus, textarea:focus { outline: 2px solid var(--ohd-red);
    outline-offset: -1px; }
  details.alt { margin-top: 1.5rem; border-top: 1px solid #e5e5ea;
    padding-top: 0.75rem; }
  details.alt summary { cursor: pointer; font-size: 0.875rem;
    color: var(--ohd-red); }
  button { width: 100%; margin-top: 1.5rem; padding: 0.7rem; font-size: 1rem;
    font-weight: 600; color: #fff; background: var(--ohd-red); border: 0;
    border-radius: 8px; cursor: pointer; }
  button:hover { filter: brightness(0.93); }
  .error { background: #fdecee; color: #b3121f; border: 1px solid #f5c2c7;
    border-radius: 8px; padding: 0.6rem 0.7rem; font-size: 0.9rem;
    margin-bottom: 1rem; }
  .muted { color: #6e6e73; font-size: 0.875rem; }
  .links { margin-top: 1.25rem; font-size: 0.875rem; text-align: center; }
  a { color: var(--ohd-red); text-decoration: none; }
  a:hover { text-decoration: underline; }
  .recovery { font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    background: #f5f5f7; border: 1px solid #d2d2d7; border-radius: 8px;
    padding: 1rem; font-size: 0.95rem; line-height: 1.7; letter-spacing: 0.06em;
    white-space: pre-wrap; word-break: break-all; margin: 1rem 0; }
  .warn { color: #b3121f; font-weight: 600; font-size: 0.9rem; }
</style>"#;

/// Wrap page `body` in the shared document shell.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title>{STYLE}</head><body>{body}</body></html>",
        title = escape(title),
    )
}

/// Render the hidden inputs that carry the authorize-flow parameters
/// through a form POST.
fn flow_fields(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    nonce: Option<&str>,
    code_challenge: &str,
) -> String {
    let mut f = format!(
        "<input type=\"hidden\" name=\"client_id\" value=\"{}\">\
         <input type=\"hidden\" name=\"redirect_uri\" value=\"{}\">\
         <input type=\"hidden\" name=\"scope\" value=\"{}\">\
         <input type=\"hidden\" name=\"state\" value=\"{}\">\
         <input type=\"hidden\" name=\"code_challenge\" value=\"{}\">",
        escape(client_id),
        escape(redirect_uri),
        escape(scope),
        escape(state),
        escape(code_challenge),
    );
    if let Some(n) = nonce {
        f.push_str(&format!(
            "<input type=\"hidden\" name=\"nonce\" value=\"{}\">",
            escape(n)
        ));
    }
    f
}

/// The `GET /login` page. `error` renders an error banner when a prior
/// `POST /login` failed; `signup_open` adds the sign-up link;
/// `recovery_enabled` adds the recovery-code form + "forgot password?"
/// link.
///
/// The page carries two `POST /login` forms — an email/password form and a
/// recovery-code form. The recovery form is hidden behind a `<details>`
/// toggle so the password path stays the prominent default, with no
/// client JS.
#[allow(clippy::too_many_arguments)]
pub fn login_page(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    nonce: Option<&str>,
    code_challenge: &str,
    signup_open: bool,
    recovery_enabled: bool,
    error: Option<&str>,
) -> String {
    let error_banner = error
        .map(|e| format!("<div class=\"error\">{}</div>", escape(e)))
        .unwrap_or_default();
    let hidden = flow_fields(client_id, redirect_uri, scope, state, nonce, code_challenge);
    let signup_link = if signup_open {
        format!(
            "<div class=\"links\">New to OHD? <a href=\"/signup?{}\">Create an account</a></div>",
            carry_query(client_id, redirect_uri, scope, state, nonce, code_challenge),
        )
    } else {
        String::new()
    };
    let recovery_block = if recovery_enabled {
        format!(
            "<details class=\"alt\"><summary>Sign in with a recovery code</summary>\
             <form method=\"post\" action=\"/login\">{hidden}\
             <label for=\"recovery_code\">Recovery code</label>\
             <textarea id=\"recovery_code\" name=\"recovery_code\" rows=\"4\" required \
             autocomplete=\"off\" spellcheck=\"false\"></textarea>\
             <button type=\"submit\">Sign in with recovery code</button></form></details>\
             <div class=\"links\"><a href=\"/reset?{carry}\">Forgot your password?</a></div>",
            hidden = hidden,
            carry = carry_query(client_id, redirect_uri, scope, state, nonce, code_challenge),
        )
    } else {
        String::new()
    };
    let body = format!(
        "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Sign in</h1>{error_banner}\
         <form method=\"post\" action=\"/login\">{hidden}\
         <label for=\"email\">Email</label>\
         <input type=\"email\" id=\"email\" name=\"email\" autofocus autocomplete=\"username\">\
         <label for=\"password\">Password</label>\
         <input type=\"password\" id=\"password\" name=\"password\" autocomplete=\"current-password\">\
         <button type=\"submit\">Sign in</button></form>\
         {recovery_block}{signup_link}</div>",
    );
    page("Sign in — OHD Identity", &body)
}

/// The `GET /reset` page — password reset via a recovery code. The user
/// pastes their recovery code and a new password; `email` is asked for too
/// (optional — only needed if the profile has no email credential yet).
#[allow(clippy::too_many_arguments)]
pub fn reset_page(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    nonce: Option<&str>,
    code_challenge: &str,
    error: Option<&str>,
) -> String {
    let error_banner = error
        .map(|e| format!("<div class=\"error\">{}</div>", escape(e)))
        .unwrap_or_default();
    let hidden = flow_fields(client_id, redirect_uri, scope, state, nonce, code_challenge);
    let body = format!(
        "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Reset your password</h1>{error_banner}\
         <p class=\"muted\">Enter your recovery code and a new password. \
         If your account has no email yet, add one below.</p>\
         <form method=\"post\" action=\"/reset\">{hidden}\
         <label for=\"recovery_code\">Recovery code</label>\
         <textarea id=\"recovery_code\" name=\"recovery_code\" rows=\"4\" required autofocus \
         autocomplete=\"off\" spellcheck=\"false\"></textarea>\
         <label for=\"password\">New password</label>\
         <input type=\"password\" id=\"password\" name=\"password\" required autocomplete=\"new-password\">\
         <label for=\"confirm\">Confirm new password</label>\
         <input type=\"password\" id=\"confirm\" name=\"confirm\" required autocomplete=\"new-password\">\
         <label for=\"email\">Email <span class=\"muted\">(only if your account has none yet)</span></label>\
         <input type=\"email\" id=\"email\" name=\"email\" autocomplete=\"username\">\
         <button type=\"submit\">Set new password</button></form>\
         <div class=\"links\"><a href=\"/login?{carry}\">Back to sign in</a></div></div>",
        carry = carry_query(client_id, redirect_uri, scope, state, nonce, code_challenge),
    );
    page("Reset password — OHD Identity", &body)
}

/// The `/logout` confirmation page — shown when logout has no (valid)
/// `post_logout_redirect_uri` to send the browser back to.
pub fn logged_out_page() -> String {
    let body = "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Signed out</h1>\
         <p class=\"muted\">You have been signed out of OHD. Close this tab, \
         or return to the application to sign in again.</p></div>";
    page("Signed out — OHD Identity", body)
}

/// The `GET /signup` page.
#[allow(clippy::too_many_arguments)]
pub fn signup_page(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    nonce: Option<&str>,
    code_challenge: &str,
    error: Option<&str>,
) -> String {
    let error_banner = error
        .map(|e| format!("<div class=\"error\">{}</div>", escape(e)))
        .unwrap_or_default();
    let hidden = flow_fields(client_id, redirect_uri, scope, state, nonce, code_challenge);
    let body = format!(
        "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Create your account</h1>{error_banner}\
         <form method=\"post\" action=\"/signup\">{hidden}\
         <label for=\"email\">Email</label>\
         <input type=\"email\" id=\"email\" name=\"email\" required autofocus autocomplete=\"username\">\
         <label for=\"password\">Password</label>\
         <input type=\"password\" id=\"password\" name=\"password\" required autocomplete=\"new-password\">\
         <label for=\"confirm\">Confirm password</label>\
         <input type=\"password\" id=\"confirm\" name=\"confirm\" required autocomplete=\"new-password\">\
         <button type=\"submit\">Create account</button></form>\
         <div class=\"links\"><a href=\"/login?{carry}\">Already have an account? Sign in</a></div></div>",
        carry = carry_query(client_id, redirect_uri, scope, state, nonce, code_challenge),
    );
    page("Create account — OHD Identity", &body)
}

/// The post-sign-up confirmation page — shows the recovery code once and
/// continues the flow on submit. The recovery code is rendered into the
/// page body and is *not* re-posted; the flow resumes via the carried
/// authorize params and a server-side `continue` token.
#[allow(clippy::too_many_arguments)]
pub fn recovery_page(continue_url: &str, recovery_code: &str) -> String {
    let body = format!(
        "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Save your recovery code</h1>\
         <p class=\"muted\">This code recovers your account if you forget your \
         password. It is shown <strong>once</strong> — write it down and keep \
         it somewhere safe.</p>\
         <div class=\"recovery\">{code}</div>\
         <p class=\"warn\">OHD cannot show this code again.</p>\
         <form method=\"get\" action=\"{cont}\">\
         <button type=\"submit\">I&#39;ve saved it — continue</button></form></div>",
        code = escape(recovery_code),
        cont = escape(continue_url),
    );
    page("Recovery code — OHD Identity", &body)
}

/// A standalone error page — used when `/authorize` cannot trust the
/// `client_id` / `redirect_uri` and therefore must *not* redirect.
pub fn error_page(message: &str) -> String {
    let body = format!(
        "<div class=\"card\"><p class=\"brand\">OHD <span class=\"accent\">Identity</span></p>\
         <h1>Something went wrong</h1>\
         <div class=\"error\">{}</div>\
         <p class=\"muted\">This request could not be completed. Return to the \
         application you came from and try again.</p></div>",
        escape(message),
    );
    page("Error — OHD Identity", &body)
}

/// Build the `?...` query string that carries the authorize params across
/// a plain `GET` link (login ⇄ signup).
fn carry_query(
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    nonce: Option<&str>,
    code_challenge: &str,
) -> String {
    let mut q = form_urlencoded::Serializer::new(String::new());
    q.append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", scope)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    if let Some(n) = nonce {
        q.append_pair("nonce", n);
    }
    q.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralises_html() {
        assert_eq!(escape("<script>&\""), "&lt;script&gt;&amp;&quot;");
    }

    #[test]
    fn login_page_carries_flow_params_and_shows_signup_when_open() {
        let html = login_page(
            "cord-web",
            "https://cord.ohd.dev/cb",
            "openid email",
            "state-abc",
            Some("nonce-xyz"),
            "challenge-123",
            true,
            true,
            None,
        );
        assert!(html.contains("name=\"client_id\" value=\"cord-web\""));
        assert!(html.contains("name=\"state\" value=\"state-abc\""));
        assert!(html.contains("name=\"nonce\" value=\"nonce-xyz\""));
        assert!(html.contains("/signup?"));
        assert!(html.contains("action=\"/login\""));
    }

    #[test]
    fn login_page_hides_signup_when_closed() {
        let html = login_page(
            "cord-web", "https://x/cb", "openid", "s", None, "c", false, true, None,
        );
        assert!(!html.contains("/signup?"));
    }

    #[test]
    fn login_page_shows_recovery_when_enabled_and_hides_it_when_not() {
        let with = login_page(
            "cord-web", "https://x/cb", "openid", "s", None, "c", true, true, None,
        );
        assert!(with.contains("recovery_code"));
        assert!(with.contains("/reset?"));

        let without = login_page(
            "cord-web", "https://x/cb", "openid", "s", None, "c", true, false, None,
        );
        assert!(!without.contains("recovery_code"));
        assert!(!without.contains("/reset?"));
    }

    #[test]
    fn login_page_renders_error_banner() {
        let html = login_page(
            "cord-web", "https://x/cb", "openid", "s", None, "c", true, true,
            Some("Incorrect email or password"),
        );
        assert!(html.contains("Incorrect email or password"));
        assert!(html.contains("class=\"error\""));
    }

    #[test]
    fn reset_page_carries_flow_params() {
        let html = reset_page(
            "cord-web", "https://cord.ohd.dev/cb", "openid", "st-r", None, "ch", None,
        );
        assert!(html.contains("action=\"/reset\""));
        assert!(html.contains("name=\"state\" value=\"st-r\""));
        assert!(html.contains("recovery_code"));
    }

    #[test]
    fn logged_out_page_renders() {
        let html = logged_out_page();
        assert!(html.contains("Signed out"));
    }

    #[test]
    fn error_page_escapes_the_message() {
        let html = error_page("<bad>");
        assert!(html.contains("&lt;bad&gt;"));
        assert!(!html.contains("<bad>"));
    }
}
