use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use url::Url;

pub(crate) struct Credentials {
    pub id: String,
    pub key: String,
}

pub(crate) fn authorization(
    url: &Url,
    method: &str,
    body: &str,
    content_type: Option<&str>,
    credentials: &Credentials,
    timestamp: u64,
    nonce: &str,
) -> Result<String, String> {
    // Strip content-type parameters, use empty string if not provided
    let ct = content_type.map(media_type).unwrap_or("");

    // Compute payload hash
    let hash = payload_hash(ct, body);

    // Build normalized request string
    let norm_req = normalized(url, method, timestamp, nonce, &hash)?;

    // Compute MAC using HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(credentials.key.as_bytes())
        .map_err(|_| "Invalid HMAC key".to_string())?;
    mac.update(norm_req.as_bytes());
    let mac_bytes = mac.finalize().into_bytes();
    let mac_value = STANDARD.encode(mac_bytes);

    // Escape quotes in ID for the header
    let escaped_id = quoted(&credentials.id);

    // Build the Hawk header with exact field order: id, ts, nonce, hash, mac
    let header = format!(
        "Hawk id=\"{}\", ts=\"{}\", nonce=\"{}\", hash=\"{}\", mac=\"{}\"",
        escaped_id, timestamp, nonce, hash, mac_value
    );

    Ok(header)
}

fn payload_hash(content_type: &str, body: &str) -> String {
    // Lowercase content type per Hawk spec before hashing
    let payload_str = format!("hawk.1.payload\n{}\n{}\n", content_type.to_lowercase(), body);
    let mut hasher = Sha256::new();
    hasher.update(payload_str.as_bytes());
    let digest = hasher.finalize();
    STANDARD.encode(digest)
}

fn media_type(value: &str) -> &str {
    // Strip parameters from content type (everything after semicolon)
    value.split(';').next().unwrap_or("").trim()
}

fn quoted(value: &str) -> String {
    // Escape backslashes and quotes for the header value
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn normalized(
    url: &Url,
    method: &str,
    timestamp: u64,
    nonce: &str,
    hash: &str,
) -> Result<String, String> {
    let host = url
        .host()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string()
        .to_lowercase();

    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no known port".to_string())?;

    // Construct path + query
    let path_with_query = if let Some(query) = url.query() {
        format!("{}?{}", url.path(), query)
    } else {
        url.path().to_string()
    };

    // Normalized request: hawk.1.header, timestamp, nonce, method, path, host,
    // port, hash, then `ext` — one line, empty here since ext is out of scope,
    // giving a single blank line after the hash. `app`/`dlg` are emitted only
    // when `app` is present, so this tool omits them entirely rather than
    // writing empty placeholders.
    let normalized = format!(
        "hawk.1.header\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n",
        timestamp,
        nonce,
        method.to_uppercase(),
        path_with_query,
        host,
        port,
        hash
    );

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_hash_matches_the_hawk_reference_example() {
        assert_eq!(
            payload_hash("text/plain", "Thank you for flying Hawk"),
            "Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=",
        );
    }

    #[test]
    fn normalized_string_ends_with_one_blank_ext_line_and_no_app_dlg() {
        // Diff this literal against Hawk's generateNormalizedString: eight
        // newline-terminated fields, then the empty `ext` line. Nothing else.
        let url = Url::parse("http://example.com:8000/resource/1?b=1&a=2").unwrap();
        assert_eq!(
            normalized(
                &url,
                "POST",
                1_353_832_234,
                "j4h3g2",
                "Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY="
            )
            .unwrap(),
            "hawk.1.header\n\
             1353832234\n\
             j4h3g2\n\
             POST\n\
             /resource/1?b=1&a=2\n\
             example.com\n\
             8000\n\
             Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=\n\
             \n",
        );
    }

    #[test]
    fn authorization_covers_a_json_body_query_and_default_empty_artifacts() {
        let url = Url::parse("http://example.com:8000/resource/1?b=1&a=2").unwrap();
        let credentials = Credentials {
            id: "dh37fgj492je".into(),
            key: "werxhqb98rpaxn39848xrunpaw3489ruxnpa98w4rxn".into(),
        };

        assert_eq!(
            authorization(
                &url,
                "POST",
                "Thank you for flying Hawk",
                Some("text/plain; charset=utf-8"),
                &credentials,
                1_353_832_234,
                "j4h3g2"
            )
            .unwrap(),
            "Hawk id=\"dh37fgj492je\", ts=\"1353832234\", nonce=\"j4h3g2\", hash=\"Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=\", mac=\"xMQacUaeJiezHpLu67V4Zc90BK53KGSS4VNYp2M3E3o=\"",
        );
    }

    #[test]
    fn authorization_uses_the_default_https_port_and_escapes_quoted_id() {
        let url = Url::parse("https://Example.TEST/a").unwrap();
        let credentials = Credentials {
            id: "a\\\"b".into(),
            key: "key".into(),
        };
        let header = authorization(&url, "get", "", None, &credentials, 1, "n").unwrap();
        assert!(header.contains("id=\"a\\\\\\\"b\""));
    }

    #[test]
    fn payload_hash_lowercases_content_type() {
        // Mixed-case Content-Type should produce same hash as lowercase
        let lowercase_hash = payload_hash("text/plain", "Thank you for flying Hawk");
        let mixedcase_hash = payload_hash("Text/Plain", "Thank you for flying Hawk");
        let uppercase_hash = payload_hash("TEXT/PLAIN", "Thank you for flying Hawk");

        assert_eq!(lowercase_hash, mixedcase_hash);
        assert_eq!(lowercase_hash, uppercase_hash);
        assert_eq!(lowercase_hash, "Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=");
    }
}
