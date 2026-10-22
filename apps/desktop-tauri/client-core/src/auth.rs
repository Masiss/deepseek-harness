//! Launch-token authentication mirrors the Host's browser exchange without exposing UI concerns.

use reqwest::Url;
use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential { pub cookie: String }

pub fn authenticated_url(base: &str, token: &str) -> Result<String, Error> {
    let mut url = Url::parse(base).map_err(|source| Error::InvalidUrl { source })?;
    url.query_pairs_mut().append_pair("token", token);
    Ok(url.to_string())
}

pub fn credential_from_redirect(headers: &reqwest::header::HeaderMap) -> Result<Credential, Error> {
    // The Host may set several cookies on one response; only the session cookie
    // whose name begins with `dsh-auth-` is the credential. Picking the first
    // cookie would silently grab the wrong value when more than one is present.
    let value = headers.get_all(reqwest::header::SET_COOKIE)
        .iter()
        .find(|v| v.to_str().map(|t| t.trim_start().starts_with("dsh-auth-")).unwrap_or(false))
        .ok_or(Error::MissingCredential)?;
    let text = value.to_str().map_err(|_| Error::InvalidCredential)?;
    let cookie = text.split(';').next().filter(|v| v.contains('=')).ok_or(Error::InvalidCredential)?;
    Ok(Credential { cookie: cookie.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adds_token_without_discarding_mount() {
        assert_eq!(authenticated_url("http://host/tools/", "abc").unwrap(), "http://host/tools/?token=abc");
    }
    #[test]
    fn extracts_cookie_from_set_cookie() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(reqwest::header::SET_COOKIE, "dsh-auth-x=value; Path=/".parse().unwrap());
        assert_eq!(credential_from_redirect(&headers).unwrap().cookie, "dsh-auth-x=value");
    }
}
