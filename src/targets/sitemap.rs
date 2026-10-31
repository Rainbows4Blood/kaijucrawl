use crate::domain::*;
use quick_xml::{Reader, events::Event};
use std::io::{BufReader, Read};

#[derive(Clone, Debug)]
pub struct Entry {
    pub location: String,
    pub lastmod: Option<String>,
    pub sitemap: bool,
}
fn invalid() -> Error {
    Failure::new(
        FailureKind::Schema,
        "Invalid sitemap XML or decoded size limit exceeded",
    )
    .into()
}
/// Streaming XML and gzip; callback batches are bounded and extension locs ignored.
pub fn parse<R: Read>(source: R, mut emit: impl FnMut(Vec<Entry>) -> Result<()>) -> Result<()> {
    let mut source = BufReader::new(source);
    let prefix = std::io::BufRead::fill_buf(&mut source)?;
    let decoded: Box<dyn Read + '_> = if prefix.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::MultiGzDecoder::new(source))
    } else {
        Box::new(source)
    };
    let mut reader = Reader::from_reader(BufReader::new(decoded.take(SITEMAP_LIMIT as u64 + 1)));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut root = String::new();
    let mut root_prefix = String::new();
    let mut loc = String::new();
    let mut lastmod = String::new();
    let mut batch = Vec::with_capacity(1000);
    let mut closed = false;
    loop {
        let e = reader.read_event_into(&mut buf).map_err(|_| invalid())?;
        if reader.buffer_position() > SITEMAP_LIMIT as u64 {
            return Err(invalid());
        }
        match e {
            Event::Start(e) => {
                let mut name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if let Some((prefix, local)) = name.split_once(':') {
                        root_prefix = format!("{prefix}:");
                        name = local.into();
                    }
                    if closed || !matches!(name.as_str(), "sitemapindex" | "urlset") {
                        return Err(invalid());
                    }
                    root = name.clone();
                } else if !root_prefix.is_empty() && name.starts_with(&root_prefix) {
                    name = name[root_prefix.len()..].into();
                }
                if stack.len() > 128 {
                    return Err(invalid());
                }
                if stack.len() == 1 {
                    loc.clear();
                    lastmod.clear();
                }
                stack.push(name);
            }
            Event::Text(t) => {
                if stack.len() == 3 && matches!(stack[1].as_str(), "url" | "sitemap") {
                    let decoded = t.decode().map_err(|_| invalid())?;
                    let text = quick_xml::escape::unescape(&decoded).map_err(|_| invalid())?;
                    if stack[2] == "loc" {
                        loc.push_str(&text);
                    } else if stack[2] == "lastmod" {
                        lastmod.push_str(&text);
                    }
                }
            }
            Event::CData(t) => {
                if stack.len() == 3 {
                    let s = t.decode().map_err(|_| invalid())?;
                    if stack[2] == "loc" {
                        loc.push_str(&s);
                    } else if stack[2] == "lastmod" {
                        lastmod.push_str(&s);
                    }
                }
            }
            Event::GeneralRef(t) => {
                if stack.len() == 3 && matches!(stack[1].as_str(), "url" | "sitemap") {
                    let name = t.decode().map_err(|_| invalid())?;
                    let entity = format!("&{name};");
                    let text = quick_xml::escape::unescape(&entity).map_err(|_| invalid())?;
                    if stack[2] == "loc" {
                        loc.push_str(&text);
                    } else if stack[2] == "lastmod" {
                        lastmod.push_str(&text);
                    }
                }
            }
            Event::End(_) => {
                if stack.len() == 2 && matches!(stack[1].as_str(), "url" | "sitemap") {
                    if loc.is_empty() {
                        return Err(invalid());
                    }
                    batch.push(Entry {
                        location: std::mem::take(&mut loc),
                        lastmod: if lastmod.is_empty() {
                            None
                        } else {
                            Some(std::mem::take(&mut lastmod))
                        },
                        sitemap: root == "sitemapindex",
                    });
                    if batch.len() == 1000 {
                        emit(std::mem::replace(&mut batch, Vec::with_capacity(1000)))?;
                    }
                }
                stack.pop();
                if stack.is_empty() {
                    closed = true;
                }
            }
            Event::DocType(_) => return Err(invalid()),
            Event::Eof => break,
            Event::Empty(e) if stack.is_empty() => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let name = name.rsplit(':').next().unwrap_or("");
                if closed || !matches!(name, "sitemapindex" | "urlset") {
                    return Err(invalid());
                }
                root = name.into();
                closed = true;
            }
            _ => {}
        }
        if loc.len() > 8192 || lastmod.len() > 1024 {
            return Err(invalid());
        }
        buf.clear();
    }
    if !closed || !stack.is_empty() || root.is_empty() {
        return Err(invalid());
    }
    if !batch.is_empty() {
        emit(batch)?;
    }
    Ok(())
}
