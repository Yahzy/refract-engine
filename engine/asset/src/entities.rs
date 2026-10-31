//! Entities as text: one `key<TAB>value` pair per line, a blank line between
//! entities. Keys may repeat (outputs, for instance). Values never contain
//! tabs or line breaks.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entity {
    pub pairs: Vec<(String, String)>,
}

impl Entity {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn classname(&self) -> &str {
        self.get("classname").unwrap_or("")
    }
}

pub fn parse(text: &str) -> Vec<Entity> {
    let mut entities = Vec::new();
    let mut current = Entity::default();
    for line in text.lines() {
        match line.split_once('\t') {
            Some((key, value)) => current.pairs.push((key.to_owned(), value.to_owned())),
            None if !current.pairs.is_empty() => entities.push(std::mem::take(&mut current)),
            None => {}
        }
    }
    if !current.pairs.is_empty() {
        entities.push(current);
    }
    entities
}

pub fn write<'a>(entities: impl IntoIterator<Item = &'a Entity>) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut text = String::new();
    for entity in entities {
        for (key, value) in &entity.pairs {
            text.push_str(&clean(key));
            text.push('\t');
            text.push_str(&clean(value));
            text.push('\n');
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        let entities = vec![
            Entity { pairs: vec![pair("classname", "worldspawn"), pair("skyname", "sky_day01_01")] },
            Entity { pairs: vec![pair("classname", "logic_relay"), pair("OnTrigger", "a,b"), pair("OnTrigger", "c\td")] },
        ];
        let parsed = parse(&write(&entities));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], entities[0]);
        assert_eq!(parsed[1].classname(), "logic_relay");
        assert_eq!(parsed[1].pairs[2].1, "c d");
    }
}
