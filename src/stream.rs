/// Split content into chunks of at most `chunk_size` characters.
/// A `chunk_size` of 0 is treated as 1.
pub(crate) fn chunk_content(content: &str, chunk_size: usize) -> Vec<String> {
    let chunk_size = chunk_size.max(1);
    if content.is_empty() {
        return Vec::new();
    }
    content
        .chars()
        .collect::<Vec<_>>()
        .chunks(chunk_size)
        .map(|c| c.iter().collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_chunk_content() {
        let chunks = chunk_content("Hello, world!", 5);
        assert_eq!(chunks, vec!["Hello", ", wor", "ld!"]);
    }

    #[test]
    fn should_chunk_content_exact_size() {
        let chunks = chunk_content("abcdef", 3);
        assert_eq!(chunks, vec!["abc", "def"]);
    }

    #[test]
    fn should_handle_content_smaller_than_chunk() {
        let chunks = chunk_content("short", 20);
        assert_eq!(chunks, vec!["short"]);
    }

    #[test]
    fn should_handle_empty_content() {
        let chunks = chunk_content("", 20);
        assert!(chunks.is_empty());
    }

    #[test]
    fn should_handle_single_char_chunks() {
        let chunks = chunk_content("abc", 1);
        assert_eq!(chunks, vec!["a", "b", "c"]);
    }

    #[test]
    fn should_handle_multibyte_characters() {
        let chunks = chunk_content("héllo", 3);
        assert_eq!(chunks, vec!["hél", "lo"]);
    }

    #[test]
    fn should_treat_zero_chunk_size_as_one() {
        let chunks = chunk_content("abc", 0);
        assert_eq!(chunks, vec!["a", "b", "c"]);
    }
}
