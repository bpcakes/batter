//! The module tree of the crate: which source files, and which parts of them,
//! are production code.

use std::{fs, path::Path};

use super::{detector::is_identifier_part, literals::sql_literals};

/// `(path relative to src, production text)` for `lib.rs` and every module it
/// declares, recursively, except a module declared under `#[cfg(test)]`. An
/// inline `#[cfg(test)] mod name { .. }` is blanked from its file's text.
pub(super) fn production_sources(source: &Path) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    let mut pending = vec!["lib.rs".to_owned()];
    while let Some(file) = pending.pop() {
        let text = fs::read_to_string(source.join(&file)).unwrap();
        let (_, code) = sql_literals(&text);
        let declarations = module_declarations(&text, &code);
        let mut production = text.clone().into_bytes();
        for declaration in &declarations {
            let enclosing: Vec<&Module> = declarations
                .iter()
                .filter(|outer| {
                    outer.body.as_ref().is_some_and(|body| {
                        body.start < declaration.start && declaration.start < body.end
                    })
                })
                .collect();
            if declaration.test_only || enclosing.iter().any(|outer| outer.test_only) {
                if let Some(body) = declaration.body.clone().filter(|_| declaration.test_only) {
                    for byte in &mut production[body] {
                        if *byte != b'\n' {
                            *byte = b' ';
                        }
                    }
                }
                continue;
            }
            if declaration.body.is_none() {
                let mut nesting: Vec<&Module> = enclosing;
                nesting.sort_by_key(|outer| outer.start);
                let nesting: Vec<&str> = nesting.iter().map(|outer| outer.name.as_str()).collect();
                pending.push(child_file(
                    source,
                    &file,
                    &nesting,
                    &declaration.name,
                    declaration.path.as_deref(),
                ));
            }
        }
        sources.push((file, String::from_utf8(production).unwrap()));
    }
    sources.sort();
    sources
}

/// One `mod` declaration: out of line (`mod name;`) or inline with its
/// braced body.
struct Module {
    name: String,
    start: usize,
    path: Option<String>,
    test_only: bool,
    body: Option<std::ops::Range<usize>>,
}

/// Every `mod name;` and inline `mod name { .. }` in `text`.
fn module_declarations(text: &str, code: &str) -> Vec<Module> {
    let bytes = code.as_bytes();
    let mut modules = Vec::new();
    for (start, _) in code.match_indices("mod ") {
        if start > 0 && is_identifier_part(bytes[start - 1]) {
            continue;
        }
        let name: String = code[start + 4..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let after_name = start + 4 + code[start + 4..].find(&name).unwrap_or(0) + name.len();
        let attributes = attributes_before(code, start);
        let test_only = attributes
            .iter()
            .any(|range| code[range.clone()].contains("cfg(test)"));
        let path = attributes.iter().find_map(|range| {
            let attribute = &text[range.clone()];
            let value = attribute.split_once("path")?.1.split_once('"')?.1;
            Some(value.split_once('"')?.0.to_owned())
        });
        let body = match code[after_name..].trim_start().bytes().next() {
            Some(b';') => None,
            Some(b'{') => {
                let open = after_name + code[after_name..].find('{').unwrap_or(0);
                let mut depth = 0_usize;
                let mut close = code.len();
                for (offset, byte) in bytes[open..].iter().enumerate() {
                    match byte {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = open + offset + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                Some(start..close)
            }
            _ => continue,
        };
        modules.push(Module {
            name,
            start,
            path,
            test_only,
            body,
        });
    }
    modules
}

/// The `#[...]` attributes, and any visibility, directly before `start`.
fn attributes_before(code: &str, start: usize) -> Vec<std::ops::Range<usize>> {
    let mut end = code[..start].trim_end().len();
    let before = &code[..end];
    if let Some(stripped) = before.strip_suffix(')').filter(|_| before.contains("pub(")) {
        if let Some(open) = stripped.rfind("pub(") {
            end = code[..open].trim_end().len();
        }
    } else if before.ends_with("pub") {
        end = code[..end - 3].trim_end().len();
    }
    let mut attributes = Vec::new();
    while code[..end].ends_with(']') {
        let Some(open) = code[..end].rfind("#[") else {
            break;
        };
        attributes.push(open..end);
        end = code[..open].trim_end().len();
    }
    attributes
}

/// The file of module `name` declared in `file`, inside the inline modules
/// `nesting`, relative to `source`.
fn child_file(
    source: &Path,
    file: &str,
    nesting: &[&str],
    name: &str,
    path: Option<&str>,
) -> String {
    let parent = Path::new(file).parent().unwrap_or_else(|| Path::new(""));
    let mut directory = if file == "lib.rs" || file.ends_with("/mod.rs") {
        parent.to_path_buf()
    } else {
        Path::new(file).with_extension("")
    };
    for outer in nesting {
        directory = directory.join(outer);
    }
    if let Some(path) = path {
        let base = if nesting.is_empty() {
            parent.to_path_buf()
        } else {
            directory
        };
        return base.join(path).to_str().unwrap().replace('\\', "/");
    }
    let flat = directory.join(format!("{name}.rs"));
    let nested = directory.join(name).join("mod.rs");
    let chosen = if source.join(&flat).exists() {
        flat
    } else {
        nested
    };
    chosen.to_str().unwrap().replace('\\', "/")
}
