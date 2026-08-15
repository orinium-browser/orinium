//! System locale discovery shared by HTTP and browser-facing JavaScript APIs.

const FALLBACK_LANGUAGE: &str = "en-US";

/// Returns the user's primary language as a browser-compatible BCP 47 tag.
pub fn preferred_language() -> String {
    system_language()
        .and_then(|language| normalize_language_tag(&language))
        .unwrap_or_else(|| FALLBACK_LANGUAGE.to_string())
}

/// Builds a conservative `Accept-Language` value from the primary language.
pub fn accept_language_header() -> String {
    let primary = preferred_language();
    let base = primary.split('-').next().unwrap_or(&primary);
    if base.eq_ignore_ascii_case(&primary) {
        primary
    } else {
        format!("{primary},{base};q=0.9,en;q=0.8")
    }
}

fn normalize_language_tag(language: &str) -> Option<String> {
    let language = language
        .split(['.', '@'])
        .next()
        .unwrap_or(language)
        .replace('_', "-");
    if language.is_empty()
        || language.eq_ignore_ascii_case("c")
        || language.eq_ignore_ascii_case("posix")
        || !language
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    Some(language)
}

#[cfg(target_os = "windows")]
fn system_language() -> Option<String> {
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, locale_name_count: i32) -> i32;
    }

    let mut buffer = [0_u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: `buffer` is writable for the reported element count and remains
    // alive until the returned UTF-16 slice has been copied into a Rust String.
    let length =
        unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), LOCALE_NAME_MAX_LENGTH as i32) };
    (length > 1).then(|| String::from_utf16_lossy(&buffer[..length as usize - 1]))
}

#[cfg(not(target_os = "windows"))]
fn system_language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_platform_locale_forms() {
        assert_eq!(normalize_language_tag("ja_JP.UTF-8"), Some("ja-JP".into()));
        assert_eq!(
            normalize_language_tag("en_US@calendar"),
            Some("en-US".into())
        );
        assert_eq!(normalize_language_tag("C"), None);
    }
}
