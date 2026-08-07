//! Markdown helpers shared by the notes and learning readers.

use std::path::PathBuf;

use gpui::{IntoElement, div, img, prelude::*, px, relative};
use gpui_component::text::{MarkdownNode, TextView, markdown_ast};

const IMAGE_NODE: &str = "ruiz-image";

/// Renders normal Markdown plus Ruiz's private local-image marker.
///
/// The marker is emitted by the optional vision import stage. Keeping the
/// image path in a custom block lets GPUI load it as a filesystem resource,
/// while regular Markdown image URLs remain handled by TextView as usual.
pub fn markdown_with_local_images(
    id: impl Into<gpui::ElementId>,
    source: impl Into<gpui::SharedString>,
) -> TextView {
    TextView::markdown(id, source)
        .markdown_block_parser(|node, _| {
            let markdown_ast::Node::Html(html) = node else {
                return None;
            };
            parse_image_marker(&html.value)
                .map(|path| MarkdownNode::new(IMAGE_NODE, path.to_string()))
        })
        .markdown_block_renderer(IMAGE_NODE, |node, _window, _cx| {
            let Some(path) = node.data::<String>() else {
                return div().into_any_element();
            };
            div()
                .w_full()
                .py_3()
                .child(
                    img(PathBuf::from(path))
                        .max_w(relative(1.))
                        .max_h(px(560.))
                        .object_fit(gpui::ObjectFit::Contain),
                )
                .into_any_element()
        })
}

fn parse_image_marker(value: &str) -> Option<&str> {
    let value = value.trim();
    let path = value
        .strip_prefix("<!-- ruiz-image:")?
        .strip_suffix("-->")?
        .trim();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::parse_image_marker;

    #[test]
    fn parses_local_image_marker() {
        assert_eq!(
            parse_image_marker("<!-- ruiz-image: /tmp/a b.png -->"),
            Some("/tmp/a b.png")
        );
        assert_eq!(parse_image_marker("<p>not a marker</p>"), None);
    }
}
