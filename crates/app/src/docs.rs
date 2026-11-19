//! The user guide (`docs/*.md`, translated in `docs/<tag>/`) built into the app: Help's Guide
//! draws its pages in the language shown from the little Markdown they use (a title, sections,
//! paragraphs, bullets, a table, `code` and links between pages), so the guide reads the same
//! offline as in the repository.

use crate::theme;
use eframe::egui::{self, Color32, FontId, text::LayoutJob};

/// A guide's pages, by file name, the index first.
type Guide = [(&'static str, &'static str); 10];

/// The index page's file.
pub(crate) const INDEX: &str = "README.md";

/// The guide in each language it is written in, by `tr::LANGUAGES` tag: English first.
const GUIDES: [(&str, Guide); 2] = [
    (
        "en",
        [
            ("README.md", include_str!("../docs/README.md")),
            ("install.md", include_str!("../docs/install.md")),
            ("first-board.md", include_str!("../docs/first-board.md")),
            ("schematic.md", include_str!("../docs/schematic.md")),
            ("parts.md", include_str!("../docs/parts.md")),
            ("board.md", include_str!("../docs/board.md")),
            ("make.md", include_str!("../docs/make.md")),
            ("analysis.md", include_str!("../docs/analysis.md")),
            ("ai.md", include_str!("../docs/ai.md")),
            ("kicad.md", include_str!("../docs/kicad.md")),
        ],
    ),
    (
        "pt-BR",
        [
            ("README.md", include_str!("../docs/pt-BR/README.md")),
            ("install.md", include_str!("../docs/pt-BR/install.md")),
            ("first-board.md", include_str!("../docs/pt-BR/first-board.md")),
            ("schematic.md", include_str!("../docs/pt-BR/schematic.md")),
            ("parts.md", include_str!("../docs/pt-BR/parts.md")),
            ("board.md", include_str!("../docs/pt-BR/board.md")),
            ("make.md", include_str!("../docs/pt-BR/make.md")),
            ("analysis.md", include_str!("../docs/pt-BR/analysis.md")),
            ("ai.md", include_str!("../docs/pt-BR/ai.md")),
            ("kicad.md", include_str!("../docs/pt-BR/kicad.md")),
        ],
    ),
];

/// The guide in the language shown, else in English.
pub(crate) fn pages() -> &'static Guide {
    let tag = crate::tr::shown().tag;
    &GUIDES.iter().find(|g| g.0 == tag).unwrap_or(&GUIDES[0]).1
}

/// A piece of a page; bullets and paragraphs with their indent (two spaces a step).
#[derive(Debug, PartialEq)]
enum Block {
    Title(String),
    Section(String),
    Text(usize, String),
    Bullet(usize, String),
    Row(Vec<String>),
}

/// A page's pieces: a line goes on the bullet or paragraph before it until a blank line.
fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = vec![];
    // Whether the last bullet or paragraph takes the next line.
    let mut open = false;
    for line in text.lines() {
        let t = line.trim_start();
        let indent = (line.len() - t.len()) / 2;
        let block = if t.is_empty() {
            None
        } else if let Some(h) = line.strip_prefix("# ") {
            Some(Block::Title(h.into()))
        } else if let Some(h) = line.strip_prefix("## ") {
            Some(Block::Section(h.into()))
        } else if t.starts_with('|') {
            let cells: Vec<String> = t.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            // The line under a table's head is no row.
            (!cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))).then_some(Block::Row(cells))
        } else if let Some(b) = t.strip_prefix("- ") {
            Some(Block::Bullet(indent, b.into()))
        } else if let (true, Some(Block::Bullet(_, s) | Block::Text(_, s))) = (open, out.last_mut()) {
            s.push(' ');
            s.push_str(t);
            continue;
        } else {
            Some(Block::Text(indent, t.into()))
        };
        open = matches!(block, Some(Block::Bullet(..) | Block::Text(..)));
        out.extend(block);
    }
    out
}

/// `text` laid out at `size` in `color`, its `code` in mono and its links' words in copper; with
/// the pages its links go to.
fn inline(text: &str, size: f32, color: Color32) -> (LayoutJob, Vec<String>) {
    let (mut job, mut links, mut rest) = (LayoutJob::default(), vec![], text);
    let add = |job: &mut LayoutJob, s: &str, font: FontId, color| job.append(s, 0.0, egui::TextFormat::simple(font, color));
    while !rest.is_empty() {
        let next = rest.find(['`', '[']).unwrap_or(rest.len());
        add(&mut job, &rest[..next], FontId::proportional(size), color);
        rest = &rest[next..];
        if let Some(code) = rest.strip_prefix('`') {
            let end = code.find('`').unwrap_or(code.len());
            add(&mut job, &code[..end], FontId::monospace(size - 0.5), theme::TEXT);
            rest = code.get(end + 1..).unwrap_or_default();
        } else if let Some((words, after)) = rest.strip_prefix('[').and_then(|r| r.split_once("](")) {
            let (to, after) = after.split_once(')').unwrap_or((after, ""));
            add(&mut job, words, FontId::proportional(size), theme::COPPER);
            links.push(to.to_string());
            rest = after;
        } else if !rest.is_empty() {
            add(&mut job, &rest[..1], FontId::proportional(size), color);
            rest = &rest[1..];
        }
    }
    (job, links)
}

/// The page `file` names, as the guides name it (each the same).
fn page_named(file: &str) -> Option<&'static str> {
    GUIDES[0].1.iter().map(|&(f, _)| f).find(|&f| f == file)
}

/// A table's rows, its head first.
fn table(ui: &mut egui::Ui, rows: Vec<Vec<String>>) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing([18.0, 2.0]).min_row_height(18.0).show(ui, |ui| {
        for (k, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let color = if k == 0 {
                    theme::QUIET
                } else if c == 0 {
                    theme::TEXT
                } else {
                    theme::TEXT_2
                };
                ui.label(inline(cell, 12.5, color).0);
            }
            ui.end_row();
        }
    });
}

/// Draws a page; returns the page a click on a bullet that opens with a link asks for.
pub(crate) fn show(ui: &mut egui::Ui, text: &str) -> Option<&'static str> {
    let (mut go, mut rows) = (None, vec![]);
    let indented = |ui: &mut egui::Ui, indent: usize| ui.add_space(indent as f32 * 16.0);
    // `None` after the last block draws a table that ends the page.
    for block in blocks(text).into_iter().map(Some).chain([None]) {
        if let Some(Block::Row(cells)) = block {
            rows.push(cells);
            continue;
        }
        if !rows.is_empty() {
            table(ui, std::mem::take(&mut rows));
        }
        match block {
            Some(Block::Title(t)) => {
                ui.label(inline(&t, 19.0, theme::TEXT).0);
                ui.add_space(4.0);
            }
            Some(Block::Section(t)) => {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(t).size(14.0).family(theme::strong()).color(theme::TEXT));
            }
            Some(Block::Text(indent, t)) => {
                ui.horizontal_top(|ui| {
                    indented(ui, indent);
                    ui.add(egui::Label::new(inline(&t, 13.0, theme::TEXT_2).0).wrap());
                });
            }
            Some(Block::Bullet(indent, t)) => {
                let (job, links) = inline(&t, 13.0, theme::TEXT_2);
                let to = links.first().filter(|_| t.starts_with('[')).and_then(|l| page_named(l));
                let row = ui.horizontal_top(|ui| {
                    indented(ui, indent);
                    ui.label(egui::RichText::new("•").color(theme::QUIET));
                    let label = egui::Label::new(job).wrap();
                    ui.add(if to.is_some() { label.sense(egui::Sense::click()) } else { label })
                });
                if to.is_some() && row.inner.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    go = to;
                }
            }
            Some(Block::Row(_)) | None => {}
        }
    }
    go
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_markdown_the_guide_uses() {
        let text =
            "# Title\nIntro that\ngoes on.\n\n## Part\n- one\n  more\n  - two\n\n  After.\n| Key | Does |\n|---|---|\n| X | Route |\n";
        assert_eq!(
            blocks(text),
            [
                Block::Title("Title".into()),
                Block::Text(0, "Intro that goes on.".into()),
                Block::Section("Part".into()),
                Block::Bullet(0, "one more".into()),
                Block::Bullet(1, "two".into()),
                Block::Text(1, "After.".into()),
                Block::Row(vec!["Key".into(), "Does".into()]),
                Block::Row(vec!["X".into(), "Route".into()]),
            ]
        );
        let (job, links) = inline("See [Board](board.md) and `X`.", 13.0, theme::TEXT_2);
        assert_eq!((job.text.as_str(), links), ("See Board and X.", vec!["board.md".to_string()]));
    }

    #[test]
    fn every_page_has_a_title_and_its_links_go_to_pages() {
        for (tag, pages) in GUIDES {
            assert!(crate::tr::LANGUAGES.iter().any(|l| l.tag == tag), "{tag} is no language");
            let dir = if tag == "en" { String::new() } else { format!("{tag}/") };
            for ((file, text), (en, _)) in pages.iter().zip(GUIDES[0].1) {
                assert_eq!(*file, en, "docs/{dir}: the pages go in English's order");
                assert!(text.starts_with("# "), "docs/{dir}{file} starts with its title");
                for block in blocks(text) {
                    if let Block::Bullet(_, t) | Block::Text(_, t) = block {
                        // A link into a folder goes to another language's guide.
                        for link in inline(&t, 13.0, theme::TEXT).1.into_iter().filter(|l| !l.contains('/')) {
                            assert!(page_named(&link).is_some(), "docs/{dir}{file} links to {link}, which isn't a page");
                        }
                    }
                }
            }
            // Every page of the folder is one of them.
            let folder = format!("{}/docs/{dir}", env!("CARGO_MANIFEST_DIR"));
            for f in std::fs::read_dir(folder).unwrap().map(|f| f.unwrap()).filter(|f| f.file_type().unwrap().is_file()) {
                let name = f.file_name().into_string().unwrap();
                assert!(page_named(&name).is_some(), "docs/{dir}{name} isn't in the guide");
            }
        }
    }
}
