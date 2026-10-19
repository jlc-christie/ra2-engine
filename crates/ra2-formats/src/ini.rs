//! The game's INI dialect: `[Section]` headers, `key=value` lines, `;` comments.
//! Section and key lookups ignore case.

use std::collections::HashMap;

pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
    /// Each section's place, by its name in lower case; the first where a name
    /// comes twice.
    index: HashMap<String, usize>,
}

impl Ini {
    pub fn parse(data: &[u8]) -> Self {
        let text = String::from_utf8_lossy(data);
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                sections.push((name.trim().to_owned(), Vec::new()));
            } else if let (Some((key, value)), Some(section)) =
                (line.split_once('='), sections.last_mut())
            {
                section.1.push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        let mut index = HashMap::with_capacity(sections.len());
        for (at, (name, _)) in sections.iter().enumerate() {
            index.entry(name.to_ascii_lowercase()).or_insert(at);
        }
        Self { sections, index }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_ascii_lowercase()).copied()
    }

    /// Lays another file's sections over this one's: a key already here takes the
    /// new value, others are added, and sections not here are added whole.
    pub fn overlay(&mut self, other: &Ini) {
        for (name, entries) in &other.sections {
            let at = match self.position(name) {
                Some(at) => at,
                None => {
                    self.sections.push((name.clone(), Vec::new()));
                    self.index.insert(name.to_ascii_lowercase(), self.sections.len() - 1);
                    self.sections.len() - 1
                }
            };
            for (key, value) in entries {
                match self.sections[at].1.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    Some(entry) => entry.1 = value.clone(),
                    None => self.sections[at].1.push((key.clone(), value.clone())),
                }
            }
        }
    }

    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(name, _)| name.as_str())
    }

    /// The entries of a section, in file order.
    pub fn section(&self, name: &str) -> &[(String, String)] {
        self.position(name).map_or(&[], |at| self.sections[at].1.as_slice())
    }

    /// A value by the key spelt exactly, capitals included, as the game itself
    /// looks keys up.
    // Keys are matched on their raw bytes.
    pub fn get_exact(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section).iter().find(|(k, _)| k == key).map(|(_, value)| value.as_str())
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Ini;

    #[test]
    fn sections_are_found_by_any_case_and_the_first_of_a_name_wins() {
        let mut ini = Ini::parse(b"[Alpha]\nA=1\n[beta]\nB=2\n[ALPHA]\nA=3\n");
        assert_eq!(ini.get("alpha", "a"), Some("1"));
        assert_eq!(ini.get("BETA", "B"), Some("2"));
        ini.overlay(&Ini::parse(b"[Gamma]\nC=4\n[alpha]\nA=5\n"));
        assert_eq!(ini.get("gamma", "c"), Some("4"));
        assert_eq!(ini.get("Alpha", "A"), Some("5"));
        assert_eq!(ini.section("missing"), &[]);
    }
}
