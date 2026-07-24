//! Placeholder substitution for the `.ini.sample` templates, reproducing the
//! bash `Configure`'s `sed` step — but with literal string replacement.
//!
//! The bash used `sed`, so it had to escape `\`, `&`, and `/` in the password
//! (see Configure lines 175-178). We use `String::replace`, which substitutes
//! the literal value, so **no such escaping is needed** — it is strictly safer.
//! The only real constraint is that a value must be valid on a single `.ini`
//! line; embedded newlines/control chars are rejected upstream at input
//! validation, not mangled here.

/// The four sentinels the bash `sed` step replaces (Configure line 184).
pub struct Substitutions<'a> {
    pub interface: &'a str,
    pub elasticsearch: &'a str,
    pub password: &'a str,
    pub install_dir: &'a str,
}

/// Render a sample by literal placeholder replacement.
///
/// Order matches the bash `sed` invocation (interface, elasticsearch, password,
/// install_dir) so behavior is identical even in the pathological case where a
/// substituted value happens to contain a later sentinel.
pub fn render(sample: &str, subs: &Substitutions) -> String {
    sample
        .replace("ARKIME_INTERFACE", subs.interface)
        .replace("ARKIME_ELASTICSEARCH", subs.elasticsearch)
        .replace("ARKIME_PASSWORD", subs.password)
        .replace("ARKIME_INSTALL_DIR", subs.install_dir)
}

/// How to encode the `user:password` in `elasticsearchBasicAuth`.
///
/// The bash `Configure` writes plaintext `user:pass` (Configure line 187), so
/// `Plaintext` is the default for exact parity. Arkime also accepts a base64
/// form; `Base64` is offered because the user asked, but note base64 is *not*
/// encryption — it is trivially reversible obfuscation, not a security control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BasicAuthEncoding {
    #[default]
    Plaintext,
    Base64,
}

/// Uncomment and fill the `# elasticsearchBasicAuth=` line, matching the bash
/// `sed -i` on Configure line 187. Only call this when a user was supplied.
///
/// Mutates `config` in place. If the sample has no `# elasticsearchBasicAuth=`
/// marker (unexpected), the config is left unchanged, mirroring `sed`.
pub fn inject_basic_auth(config: &mut String, user: &str, pass: &str, enc: BasicAuthEncoding) {
    let value = basic_auth_value(user, pass, enc);
    *config = config.replace(
        "# elasticsearchBasicAuth=",
        &format!("elasticsearchBasicAuth={value}"),
    );
}

/// The `elasticsearchBasicAuth` value alone (`user:pass` or its base64 form).
/// Shared by the native `.ini` injection and the docker `ARKIME__*` env output.
pub fn basic_auth_value(user: &str, pass: &str, enc: BasicAuthEncoding) -> String {
    match enc {
        BasicAuthEncoding::Plaintext => format!("{user}:{pass}"),
        BasicAuthEncoding::Base64 => base64_encode(format!("{user}:{pass}").as_bytes()),
    }
}

/// Minimal standard-alphabet base64 (RFC 4648), no line wrapping. Kept local to
/// avoid pulling a crate for a handful of bytes.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_all_sentinels() {
        let sample = "interface=ARKIME_INTERFACE\n\
                      elasticsearch=ARKIME_ELASTICSEARCH\n\
                      passwordSecret=ARKIME_PASSWORD\n\
                      pcapDir=ARKIME_INSTALL_DIR/raw\n";
        let out = render(
            sample,
            &Substitutions {
                interface: "eth0;eth1",
                elasticsearch: "https://os:9200",
                password: "s3cr3t",
                install_dir: "/opt/arkime",
            },
        );
        assert_eq!(
            out,
            "interface=eth0;eth1\n\
             elasticsearch=https://os:9200\n\
             passwordSecret=s3cr3t\n\
             pcapDir=/opt/arkime/raw\n"
        );
    }

    #[test]
    fn render_password_with_sed_special_chars_is_literal() {
        // Chars the bash had to escape for sed (\ & /) are substituted literally.
        let sample = "passwordSecret=ARKIME_PASSWORD\n";
        let out = render(
            sample,
            &Substitutions {
                interface: "",
                elasticsearch: "",
                password: r"a\b&c/d",
                install_dir: "",
            },
        );
        assert_eq!(out, "passwordSecret=a\\b&c/d\n");
    }

    #[test]
    fn inject_basic_auth_plaintext_uncomments_line() {
        let mut config = String::from("elasticsearch=https://os:9200\n# elasticsearchBasicAuth=\n");
        inject_basic_auth(&mut config, "admin", "pass", BasicAuthEncoding::Plaintext);
        assert_eq!(
            config,
            "elasticsearch=https://os:9200\nelasticsearchBasicAuth=admin:pass\n"
        );
    }

    #[test]
    fn inject_basic_auth_base64_matches_known_vector() {
        // base64("admin:pass") == "YWRtaW46cGFzcw=="
        let mut config = String::from("# elasticsearchBasicAuth=\n");
        inject_basic_auth(&mut config, "admin", "pass", BasicAuthEncoding::Base64);
        assert_eq!(config, "elasticsearchBasicAuth=YWRtaW46cGFzcw==\n");
    }

    #[test]
    fn inject_basic_auth_noop_without_marker() {
        let mut config = String::from("elasticsearch=https://os:9200\n");
        let before = config.clone();
        inject_basic_auth(&mut config, "admin", "pass", BasicAuthEncoding::Plaintext);
        assert_eq!(config, before);
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
