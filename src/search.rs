//! History search: what the search field of the commit list asks for, and
//! which commits match it.
//!
//! A word matches the message, the author's name or email, or the start of
//! the SHA. `author:` matches the author only, and `path:` keeps the commits
//! that change a file or folder (a Git pathspec, so `path:*.md` works).
//! Quotes keep words together: `"word diff"`, `author:"Jane Doe"`. A commit
//! must match each word and each `author:`, and one of the paths.

use crate::git::Commit;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// In lower case.
    words: Vec<String>,
    /// In lower case.
    authors: Vec<String>,
    pub paths: Vec<String>,
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let mut q = Query::default();
        for term in terms(text) {
            if let Some(a) = term.strip_prefix("author:") {
                if !a.is_empty() {
                    q.authors.push(a.to_lowercase());
                }
            } else if let Some(p) = term.strip_prefix("path:") {
                // Paths are from the top of the repository.
                let p = p.trim_start_matches('/');
                if !p.is_empty() {
                    q.paths.push(p.to_string());
                }
            } else {
                q.words.push(term.to_lowercase());
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.authors.is_empty() && self.paths.is_empty()
    }

    /// Words and authors need the messages and emails (`git::messages`).
    /// Paths need only the log: Git keeps the commits that change them.
    pub fn needs_messages(&self) -> bool {
        !self.words.is_empty() || !self.authors.is_empty()
    }

    /// Whether `c` matches the words and authors. `message` is the author's
    /// email and the whole message of `c`.
    pub fn matches(&self, c: &Commit, message: Option<&(String, String)>) -> bool {
        let (email, body) = message.map_or(("", ""), |(e, b)| (e.as_str(), b.as_str()));
        let (author, email) = (c.author.to_lowercase(), email.to_lowercase());
        // The subject is the start of the message, when the message loaded.
        let text = if body.is_empty() { &c.subject } else { body }.to_lowercase();
        let by = |a: &String| author.contains(a.as_str()) || email.contains(a.as_str());
        self.authors.iter().all(by)
            && self.words.iter().all(|w| {
                text.contains(w.as_str())
                    || by(w)
                    || (w.len() >= 4
                        && w.chars().all(|c| c.is_ascii_hexdigit())
                        && c.sha.starts_with(w.as_str()))
            })
    }
}

/// The terms of `text`, split at spaces outside quotes, without the quotes.
fn terms(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut term = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !term.is_empty() {
                    out.push(std::mem::take(&mut term));
                }
            }
            c => term.push(c),
        }
    }
    if !term.is_empty() {
        out.push(term);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, author: &str, subject: &str) -> Commit {
        Commit {
            sha: sha.into(),
            parents: vec![],
            author: author.into(),
            time: 0,
            subject: subject.into(),
            refs: vec![],
        }
    }

    fn message(email: &str, text: &str) -> Option<(String, String)> {
        Some((email.into(), text.into()))
    }

    #[test]
    fn parse_terms_quotes_and_prefixes() {
        let q = Query::parse(r#"  Fix "word diff" author:"Jane Doe" path:/src/app path: author:"#);
        assert_eq!(q.words, ["fix", "word diff"]);
        assert_eq!(q.authors, ["jane doe"]);
        assert_eq!(q.paths, ["src/app"]);
        assert!(Query::parse("  \"\" ").is_empty());
        assert!(!Query::parse("path:src").needs_messages());
    }

    #[test]
    fn words_match_message_author_or_sha() {
        let c = commit("abc123ff", "Jane Doe", "feat: add search");
        let m = message(
            "jane@example.com",
            "feat: add search\n\nFilter the History.\n",
        );
        let hit = |q: &str| Query::parse(q).matches(&c, m.as_ref());
        assert!(hit("SEARCH"));
        assert!(hit("history"), "the body counts");
        assert!(hit("jane"));
        assert!(hit("example.com"));
        assert!(hit("abc1"));
        assert!(!hit("abc"), "a SHA needs 4 characters");
        assert!(!hit("123f"), "a SHA matches from its start");
        assert!(hit("jane search"));
        assert!(!hit("jane rebase"), "each word must match");
        assert!(hit("author:doe"));
        assert!(!hit("author:search"), "author: does not read the message");
        assert!(hit(r#""add search""#));
        assert!(!hit(r#""search add""#));
    }

    #[test]
    fn subject_without_message() {
        let c = commit("abc123ff", "Jane Doe", "feat: add search");
        assert!(Query::parse("search").matches(&c, None));
    }
}
