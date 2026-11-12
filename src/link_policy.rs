//! Optional known-tracker removal. Original message bytes are never changed.
//! This helper is for explicit Open/Copy-link/fetch actions, not message copy.

/// Return the original byte-for-byte unless an explicitly listed tracking key
/// is removed. Preserve all unrecognized fields and all recognized signed URLs.
pub fn without_tracking(value: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(value) else {
        return value.into();
    };
    if !matches!(parsed.scheme(), "https" | "http")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return value.into();
    }
    let signed = parsed.query_pairs().any(|(key, _)| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "sig"
                | "signature"
                | "x-amz-signature"
                | "x-goog-signature"
                | "oauth_signature"
                | "policy"
                | "key-pair-id"
                | "awsaccesskeyid"
                | "token"
                | "access_token"
                | "auth"
        )
    });
    if signed {
        return value.into();
    }
    let (resource, fragment) = value
        .split_once('#')
        .map_or((value, None), |(resource, fragment)| {
            (resource, Some(fragment))
        });
    let Some((base, query)) = resource.split_once('?') else {
        return value.into();
    };
    let mut removed = false;
    let fields: Vec<_> = query
        .split('&')
        .filter(|field| {
            let raw_key = field.split_once('=').map_or(*field, |(key, _)| key);
            // Use the same URL query decoder as the URL parser, but keep the raw
            // surviving fields so signatures/encoding/order are not rewritten.
            let mut key_url = parsed.clone();
            key_url.set_query(Some(raw_key));
            let key = key_url
                .query_pairs()
                .next()
                .map(|(key, _)| key.to_ascii_lowercase())
                .unwrap_or_default();
            let tracking = key.starts_with("utm_")
                || matches!(
                    key.as_str(),
                    "fbclid"
                        | "gclid"
                        | "dclid"
                        | "msclkid"
                        | "twclid"
                        | "mc_cid"
                        | "mc_eid"
                        | "igshid"
                        | "_hsenc"
                        | "_hsmi"
                );
            removed |= tracking;
            !tracking
        })
        .collect();
    if !removed {
        return value.into();
    }
    let mut clean = base.to_owned();
    if !fields.is_empty() {
        clean.push('?');
        clean.push_str(&fields.join("&"));
    }
    if let Some(fragment) = fragment {
        clean.push('#');
        clean.push_str(fragment);
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surviving_duplicates_empty_values_and_escaped_signed_keys_stay_exact() {
        assert_eq!(
            without_tracking("https://example.com/?a=1&utm_source=x&a=2&empty=&bare#q"),
            "https://example.com/?a=1&a=2&empty=&bare#q"
        );
        assert_eq!(
            without_tracking("https://example.com/?UTM_SOURCE=x&x=%2f%2F+%20&fbclid=1"),
            "https://example.com/?x=%2f%2F+%20"
        );
        for url in [
            "https://example.com/?%73ig=exact&utm_source=x",
            "https://example.com/?utm_source=x&access_token=secret",
            "https://example.com/?utm_source=x&policy=encoded",
        ] {
            assert_eq!(without_tracking(url), url);
        }
    }
    #[test]
    fn strips_only_known_keys_preserving_real_parameters_and_encoding() {
        assert_eq!(
            without_tracking(
                "https://example.com/p?q=a%20b&utm_source=chat&view=full&fbclid=secret#part"
            ),
            "https://example.com/p?q=a%20b&view=full#part"
        );
        assert_eq!(
            without_tracking("https://example.com/p?%75tm_source=chat&ID=42&ref=home"),
            "https://example.com/p?ID=42&ref=home"
        );
        assert_eq!(
            without_tracking("https://example.com/?utm_source=x#gclid=keep"),
            "https://example.com/#gclid=keep"
        );
        for original in [
            "https://example.com/?utm_source=x&X-Amz-Signature=abc",
            "https://example.com/?utm_source=x&sig=abc",
            "https://example.com/?utm_source=x&token=abc",
            "https://example.com/?q=a%20b&ref=keep#part",
            "file:///example?utm_source=x",
        ] {
            assert_eq!(without_tracking(original), original);
        }
    }
}
