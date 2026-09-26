use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// A single word or token returned by the aligner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A displayable subtitle cue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

const RIGHT_PUNCTUATION: &str = ",.!?:;%)]}，。！？：；、）】」』》";
const LEFT_PUNCTUATION: &str = "([{（【「『《";

/// Join aligner tokens using the same spacing rules as the Python pipeline.
pub fn join_tokens(tokens: &[String]) -> String {
    let mut result = String::new();

    for raw in tokens {
        let explicit_space = raw.chars().next().is_some_and(is_python_whitespace);
        let token = trim_python_whitespace(raw);
        let Some(first) = token.chars().next() else {
            continue;
        };

        if let Some(previous) = result.chars().last() {
            let needs_space = explicit_space
                || (!RIGHT_PUNCTUATION.contains(first)
                    && !LEFT_PUNCTUATION.contains(previous)
                    && !LEFT_PUNCTUATION.contains(first)
                    && !is_no_space_script(previous)
                    && !is_no_space_script(first));
            if needs_space {
                result.push(' ');
            }
        }

        result.push_str(token);
    }

    result
}

fn is_no_space_script(character: char) -> bool {
    matches!(character as u32,
        0x3040..=0x30ff | // Hiragana and Katakana
        0x3400..=0x9fff | // CJK unified ideographs
        0xf900..=0xfaff | // CJK compatibility ideographs
        0xff65..=0xff9f   // Halfwidth Katakana
    )
}

fn is_python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{001c}'..='\u{001f}')
}

fn trim_python_whitespace(text: &str) -> &str {
    text.trim_matches(is_python_whitespace)
}

/// Reattach punctuation that the official aligner tokenizer omitted.
///
/// Every recognized alphanumeric character must be covered before subtitles
/// can be committed; a partial alignment must never silently shorten ASR text.
pub fn restore_word_punctuation(words: &[Word], transcript: &str) -> Result<Vec<Word>> {
    if words.is_empty() {
        if !transcript.trim().is_empty() {
            bail!("Forced alignment returned no words for a nonempty transcript");
        }
        return Ok(Vec::new());
    }

    let mut restored = words.to_vec();
    let mut cursor = 0;

    for index in 0..restored.len() {
        let token = &restored[index].text;
        let Some((start, end)) = find_case_insensitive(transcript, cursor, token) else {
            bail!("Forced-alignment token does not match the recognized transcript: {token:?}");
        };

        let gap = &transcript[cursor..start];
        if !trim_python_whitespace(gap).is_empty() {
            if gap.chars().any(char::is_alphanumeric) {
                bail!("Forced alignment omitted recognized words before token {token:?}");
            }

            let gap_trimmed_first = trim_python_whitespace(gap).chars().next();
            if index == 0 || gap_trimmed_first.is_some_and(|ch| LEFT_PUNCTUATION.contains(ch)) {
                restored[index].text = format!("{gap}{}", restored[index].text);
            } else {
                restored[index - 1]
                    .text
                    .push_str(trim_python_whitespace(gap));
            }
        }
        cursor = end;
    }

    let tail = trim_python_whitespace(&transcript[cursor..]);
    if tail.chars().any(char::is_alphanumeric) {
        bail!("Forced alignment omitted recognized words at the end of the transcript");
    }
    if !tail.is_empty() && !tail.chars().any(char::is_alphanumeric) {
        restored
            .last_mut()
            .expect("nonempty words")
            .text
            .push_str(tail);
    }

    Ok(restored)
}

/// Build readable, ordered subtitle cues from aligned words.
pub fn cues_from_words(words: &[Word], max_chars: usize, max_duration: f64) -> Result<Vec<Cue>> {
    if max_chars < 1 || max_duration <= 0.0 {
        bail!("Subtitle limits must be positive.");
    }

    let mut cues = Vec::new();
    let mut group: Vec<&Word> = Vec::new();

    let flush = |group: &mut Vec<&Word>, cues: &mut Vec<Cue>| {
        if let (Some(first), Some(last)) = (group.first(), group.last()) {
            cues.push(Cue {
                start: first.start,
                end: last.end,
                text: join_tokens(
                    &group
                        .iter()
                        .map(|word| word.text.clone())
                        .collect::<Vec<_>>(),
                ),
            });
            group.clear();
        }
    };

    for word in words {
        if trim_python_whitespace(&word.text).is_empty() {
            continue;
        }
        if !word.start.is_finite()
            || !word.end.is_finite()
            || word.start < 0.0
            || word.end < word.start
        {
            bail!("Invalid forced-alignment word timestamps.");
        }
        if group
            .last()
            .is_some_and(|previous| word.start < previous.start)
        {
            bail!("Forced-alignment word timestamps are out of order.");
        }

        let mut proposed_tokens: Vec<String> = group.iter().map(|part| part.text.clone()).collect();
        proposed_tokens.push(word.text.clone());
        let proposed = join_tokens(&proposed_tokens);

        if let Some(first) = group.first() {
            let last = group.last().expect("group has a first word");
            if proposed.chars().count() > max_chars
                || word.end - first.start > max_duration
                || word.start - last.end > 1.0
            {
                flush(&mut group, &mut cues);
            }
        }

        group.push(word);
        if word
            .text
            .chars()
            .last()
            .is_some_and(|character| ".!?。！？".contains(character))
        {
            let text = join_tokens(
                &group
                    .iter()
                    .map(|part| part.text.clone())
                    .collect::<Vec<_>>(),
            );
            if text.chars().count() >= max_chars / 2 {
                flush(&mut group, &mut cues);
            }
        }
    }
    flush(&mut group, &mut cues);

    let mut normalized: Vec<Cue> = Vec::new();
    let mut pending_text = Vec::new();
    for cue in cues {
        if cue.end <= cue.start {
            pending_text.push(cue.text);
            continue;
        }
        let mut cue = cue;
        if !pending_text.is_empty() {
            pending_text.push(cue.text);
            cue.text = join_tokens(&pending_text);
            pending_text.clear();
        }
        if normalized
            .last()
            .is_some_and(|previous| cue.start < previous.end)
        {
            bail!("Forced alignment produced overlapping subtitle cues.");
        }
        normalized.push(cue);
    }

    if !pending_text.is_empty() {
        let Some(last) = normalized.last_mut() else {
            bail!("Forced alignment produced only zero-duration subtitle cues.");
        };
        let mut tokens = vec![last.text.clone()];
        tokens.extend(pending_text);
        last.text = join_tokens(&tokens);
    }

    Ok(normalized)
}

/// Render cues as SubRip (SRT).
pub fn render_srt(cues: &[Cue]) -> String {
    let mut output = String::new();
    for (index, cue) in cues.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&(index + 1).to_string());
        output.push('\n');
        output.push_str(&timestamp(cue.start, false));
        output.push_str(" --> ");
        output.push_str(&timestamp(cue.end, false));
        output.push('\n');
        output.push_str(&cue.text);
        output.push('\n');
    }
    output
}

/// Render cues as WebVTT.
pub fn render_vtt(cues: &[Cue]) -> String {
    let mut output = String::from("WEBVTT\n\n");
    for (index, cue) in cues.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&timestamp(cue.start, true));
        output.push_str(" --> ");
        output.push_str(&timestamp(cue.end, true));
        output.push('\n');
        output.push_str(&cue.text);
        output.push('\n');
    }
    output
}

fn timestamp(seconds: f64, vtt: bool) -> String {
    let value = seconds * 1000.0;
    let floor = value.floor();
    let fraction = value - floor;
    let rounded = if fraction == 0.5 {
        if (floor as u128).is_multiple_of(2) {
            floor
        } else {
            floor + 1.0
        }
    } else {
        value.round()
    };
    let milliseconds = rounded.max(0.0) as u128;
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds / 60_000) % 60;
    let seconds = (milliseconds / 1_000) % 60;
    let millis = milliseconds % 1_000;
    let separator = if vtt { '.' } else { ',' };
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{millis:03}")
}

fn find_case_insensitive(haystack: &str, start: usize, needle: &str) -> Option<(usize, usize)> {
    let search = haystack.get(start..)?;
    if needle.is_empty() {
        return Some((start, start));
    }

    for (relative_start, _) in search.char_indices() {
        let candidate = &search[relative_start..];
        let mut candidate_chars = candidate.char_indices();
        let mut relative_end = 0;
        let mut matched = true;

        for expected in needle.chars() {
            let Some((offset, actual)) = candidate_chars.next() else {
                matched = false;
                break;
            };
            if simple_case_fold(expected) != simple_case_fold(actual) {
                matched = false;
                break;
            }
            relative_end = offset + actual.len_utf8();
        }

        if matched {
            return Some((
                start + relative_start,
                start + relative_start + relative_end,
            ));
        }
    }
    None
}

fn simple_case_fold(character: char) -> char {
    match character {
        // Python's Unicode re.IGNORECASE includes these four non-ASCII
        // characters in the corresponding ASCII letter classes.
        '\u{0130}' | '\u{0131}' => 'i',
        '\u{017f}' => 's',
        '\u{212a}' => 'k',
        _ => {
            let mut lowercase = character.to_lowercase();
            let first = lowercase.next().unwrap_or(character);
            if lowercase.next().is_none() {
                first
            } else {
                character
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start: f64, end: f64, text: &str) -> Word {
        Word {
            start,
            end,
            text: text.to_owned(),
        }
    }

    #[test]
    fn word_timestamps_become_cues_and_render_both_formats() {
        let words = [
            word(0.1, 0.5, "Hello"),
            word(0.5, 0.9, "world"),
            word(2.0, 2.2, "again"),
        ];
        let cues = cues_from_words(&words, 20, 6.0).unwrap();
        assert_eq!(
            cues,
            [
                Cue {
                    start: 0.1,
                    end: 0.9,
                    text: "Hello world".into(),
                },
                Cue {
                    start: 2.0,
                    end: 2.2,
                    text: "again".into(),
                },
            ]
        );
        assert_eq!(
            render_srt(&cues),
            "1\n00:00:00,100 --> 00:00:00,900\nHello world\n\n2\n00:00:02,000 --> 00:00:02,200\nagain\n"
        );
        assert_eq!(
            render_vtt(&cues),
            "WEBVTT\n\n00:00:00.100 --> 00:00:00.900\nHello world\n\n00:00:02.000 --> 00:00:02.200\nagain\n"
        );
    }

    #[test]
    fn cjk_join_and_invalid_timing_match_python_baseline() {
        assert_eq!(
            join_tokens(&["你".into(), "好".into(), "world".into(), "!".into()]),
            "你好world!"
        );
        assert_eq!(
            join_tokens(&[
                "こ".into(),
                "ん".into(),
                "に".into(),
                "ち".into(),
                "は".into()
            ]),
            "こんにちは"
        );
        let aligned = [word(0.0, 0.4, "Hello"), word(0.4, 1.0, "world")];
        let restored = restore_word_punctuation(&aligned, "Hello, world!").unwrap();
        assert_eq!(
            restored
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>(),
            ["Hello,", "world!"]
        );
        let error = cues_from_words(&[word(2.0, 1.0, "bad")], 42, 6.0).unwrap_err();
        assert!(error.to_string().contains("Invalid forced-alignment"));
    }

    #[test]
    fn zero_duration_words_merge_and_overlapping_cues_are_rejected() {
        let cues = cues_from_words(&[word(0.0, 0.0, "A"), word(0.0, 1.0, "word")], 1, 6.0).unwrap();
        assert_eq!(
            cues,
            [Cue {
                start: 0.0,
                end: 1.0,
                text: "A word".into()
            }]
        );

        let error = cues_from_words(&[word(0.0, 2.0, "first"), word(1.0, 3.0, "second")], 3, 6.0)
            .unwrap_err();
        assert!(error.to_string().contains("overlapping"));
    }

    #[test]
    fn restored_punctuation_preserves_transcript_parenthesis_spacing() {
        let words = [word(0.0, 0.4, "foo"), word(0.4, 1.0, "bar")];
        let tight = restore_word_punctuation(&words, "foo(bar)").unwrap();
        let spaced = restore_word_punctuation(&words, "foo (bar)").unwrap();
        assert_eq!(
            join_tokens(&tight.iter().map(|w| w.text.clone()).collect::<Vec<_>>()),
            "foo(bar)"
        );
        assert_eq!(
            join_tokens(&spaced.iter().map(|w| w.text.clone()).collect::<Vec<_>>()),
            "foo (bar)"
        );
        assert_eq!(
            cues_from_words(&tight, 42, 6.0).unwrap()[0].text,
            "foo(bar)"
        );
        assert_eq!(
            cues_from_words(&spaced, 42, 6.0).unwrap()[0].text,
            "foo (bar)"
        );

        let case_insensitive = restore_word_punctuation(&[word(0.0, 0.2, "k")], "K!").unwrap();
        assert_eq!(case_insensitive[0].text, "k!");
    }

    #[test]
    fn limits_count_unicode_scalar_values_and_srt_rounds_half_to_even() {
        let cues = cues_from_words(
            &[
                word(0.0, 0.1, "😀"),
                word(0.1, 0.2, "你"),
                word(0.2, 0.3, "x"),
            ],
            2,
            6.0,
        )
        .unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "😀你");

        let tie = [Cue {
            start: 1.2345,
            end: 2.3455,
            text: "round".into(),
        }];
        let srt = render_srt(&tie);
        assert!(srt.contains("00:00:01,234 --> 00:00:02,346"), "{srt}");
    }

    #[test]
    fn only_zero_duration_words_and_nonpositive_limits_are_errors() {
        assert!(
            cues_from_words(&[word(0.0, 0.0, "only")], 42, 6.0)
                .unwrap_err()
                .to_string()
                .contains("only zero-duration")
        );
        assert!(cues_from_words(&[], 0, 6.0).is_err());
        assert!(cues_from_words(&[], 42, 0.0).is_err());
    }

    #[test]
    fn incomplete_alignment_never_silently_discards_recognized_words() {
        let hello = [word(0.1, 0.5, "hello")];
        assert!(restore_word_punctuation(&hello, "hello world").is_err());
        assert!(restore_word_punctuation(&hello, "missing hello").is_err());
        assert!(restore_word_punctuation(&[], "recognized speech").is_err());
        assert!(restore_word_punctuation(&hello, "different words").is_err());
        assert!(restore_word_punctuation(&hello, "hello!").is_ok());
    }
}
