//! The KeyValues text format (VMT, VDF, ...): nested `"key" "value"` pairs
//! and `"key" { ... }` blocks, `//` comments, and platform conditionals such as
//! `"$basetexture" "x" [$X360]`, evaluated for a Windows PC.
//!
//! Backslashes are kept as is: VMT paths use them as separators, not escapes.

/// A value: a string, or a block of child pairs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Block(Block),
}

/// Ordered pairs; keys are compared case-insensitively.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block(pub Vec<(String, Value)>);

impl Block {
    /// Last value of `key`, like a VMT where later lines override earlier ones.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            Value::Block(_) => None,
        }
    }

    pub fn get_block(&self, key: &str) -> Option<&Block> {
        match self.get(key)? {
            Value::Block(b) => Some(b),
            Value::Str(_) => None,
        }
    }

    /// The string pairs, in order.
    pub fn strings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().filter_map(|(k, v)| match v {
            Value::Str(s) => Some((k.as_str(), s.as_str())),
            Value::Block(_) => None,
        })
    }

    /// The block pairs, in order.
    pub fn blocks(&self) -> impl Iterator<Item = (&str, &Block)> {
        self.0.iter().filter_map(|(k, v)| match v {
            Value::Block(b) => Some((k.as_str(), b)),
            Value::Str(_) => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// A quoted or bare string.
    Str(String),
    Open,
    Close,
    /// `[$X360]`, `[!$WIN32]`...
    Condition(String),
}

/// Parses a whole file: usually one root pair, like `"LightmappedGeneric" { ... }`.
pub fn parse(text: &str) -> Block {
    let tokens = tokenize(text);
    let mut pos = 0;
    parse_block(&tokens, &mut pos)
}

fn parse_block(tokens: &[Token], pos: &mut usize) -> Block {
    let mut block = Block::default();
    while let Some(token) = tokens.get(*pos) {
        *pos += 1;
        let key = match token {
            Token::Close => break,
            Token::Str(key) => key.clone(),
            // Stray braces or conditions: skip.
            Token::Open | Token::Condition(_) => continue,
        };
        let value = match tokens.get(*pos) {
            Some(Token::Str(value)) => {
                *pos += 1;
                Value::Str(value.clone())
            }
            Some(Token::Open) => {
                *pos += 1;
                Value::Block(parse_block(tokens, pos))
            }
            // A key without a value at the end of a block.
            _ => continue,
        };
        let included = match tokens.get(*pos) {
            Some(Token::Condition(condition)) => {
                *pos += 1;
                condition_holds(condition)
            }
            _ => true,
        };
        if included {
            block.0.push((key, value));
        }
    }
    block
}

/// Evaluates a platform conditional for Windows PC.
fn condition_holds(condition: &str) -> bool {
    let condition = condition.trim().to_ascii_uppercase();
    let (negated, name) = match condition.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, condition.as_str()),
    };
    let on_this_platform = matches!(name.trim_start_matches('$'), "WIN32" | "WINDOWS" | "PC" | "DX9" | "HDR");
    on_this_platform != negated
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() || c == '\u{feff}' => {
                chars.next();
            }
            '/' if chars.clone().nth(1) == Some('/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' => {
                chars.next();
                tokens.push(Token::Open);
            }
            '}' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                tokens.push(Token::Str(s));
            }
            '[' if matches!(tokens.last(), Some(Token::Str(_)) | Some(Token::Close)) && is_condition(&chars) => {
                chars.next();
                let condition: String = chars.by_ref().take_while(|c| *c != ']').collect();
                tokens.push(Token::Condition(condition));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '"' || c == '{' || c == '}' {
                        break;
                    }
                    // A bare `[1 0 0]` vector runs to its closing bracket.
                    if c == '[' && s.is_empty() {
                        s.extend(chars.by_ref().take_while(|c| *c != ']'));
                        s.push(']');
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                tokens.push(Token::Str(s));
            }
        }
    }
    tokens
}

/// `[$X360]` or `[!$WIN32]`, as opposed to a bare vector value `[1 0 0]`.
fn is_condition(chars: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let inner: String = chars.clone().skip(1).take_while(|c| *c != ']' && *c != '\n').collect();
    let inner = inner.trim();
    inner.starts_with('$') || inner.starts_with("!$")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vmt() {
        let text = r#"
            // comment
            "VertexLitGeneric"
            {
                "$basetexture" "models\props/tree" // trailing
                $alphatest 1
                "$color" "[1 0.5 0]"
                $color2 [0.5 0.5 0.5]
                "$basetexture" "models/x360" [$X360]
                "$nocull" "1" [!$X360]
                "Proxies"
                {
                    "TextureScroll" { "texturescrollvar" "$basetexturetransform" "texturescrollrate" ".2" }
                }
            }"#;
        let root = parse(text);
        let (shader, Value::Block(body)) = &root.0[0] else { panic!() };
        assert_eq!(shader, "VertexLitGeneric");
        assert_eq!(body.get_str("$BASETEXTURE"), Some("models\\props/tree"));
        assert_eq!(body.get_str("$alphatest"), Some("1"));
        assert_eq!(body.get_str("$color"), Some("[1 0.5 0]"));
        assert_eq!(body.get_str("$color2"), Some("[0.5 0.5 0.5]"));
        assert_eq!(body.get_str("$nocull"), Some("1"));
        let proxies = body.get_block("proxies").unwrap();
        let (name, scroll) = proxies.blocks().next().unwrap();
        assert_eq!(name, "TextureScroll");
        assert_eq!(scroll.get_str("textureScrollRate"), Some(".2"));
    }
}
