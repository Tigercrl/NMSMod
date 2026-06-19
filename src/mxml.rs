use std::collections::{HashMap, HashSet};
use thiserror::Error;
use xmltree::{Element, XMLNode};

#[derive(Error, Debug)]
pub enum MxmlError {
    #[error("MXML 解析错误: {0}")]
    ParseError(String),
    #[error("MXML 写入错误: {0}")]
    WriteError(String),
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len).collect::<String>() + "..."
    }
}

/// 提取 XML 节点属性作为其唯一标识与路径片段
fn format_identity(el: &Element) -> String {
    let name = el.attributes.get("name").map(|s| s.as_str()).unwrap_or("");
    let value = el.attributes.get("value").map(|s| s.as_str()).unwrap_or("");
    let id = el.attributes.get("_id").map(|s| s.as_str()).unwrap_or("");
    format!(
        "(name:{},_id:{},value:{})",
        truncate(name, 20),
        truncate(id, 20),
        truncate(value, 20)
    )
}

/// 递归收集 MXML 中所有 Property 节点的完整路径
fn collect_paths(el: &Element, current_path: &str, paths: &mut HashSet<String>) {
    let local_key = format_identity(el);
    let next_path = if current_path.is_empty() {
        local_key
    } else {
        format!("{}/{}", current_path, local_key)
    };

    // 仅处理 Property 节点，过滤 Data 根节点
    if el.name == "Property" {
        paths.insert(next_path.clone());
    }

    for child in &el.children {
        if let XMLNode::Element(child_el) = child {
            collect_paths(child_el, &next_path, paths);
        }
    }
}

/// 递归深度合并两个 XML 元素树
fn merge_elements(base: &mut Element, extra: &Element) {
    for extra_node in &extra.children {
        if let XMLNode::Element(extra_child) = extra_node {
            let extra_key = format_identity(extra_child);

            let mut found_idx = None;
            for (idx, base_node) in base.children.iter().enumerate() {
                if let XMLNode::Element(base_child) = base_node {
                    if format_identity(base_child) == extra_key {
                        found_idx = Some(idx);
                        break;
                    }
                }
            }

            if let Some(idx) = found_idx {
                if let XMLNode::Element(base_child) = &mut base.children[idx] {
                    merge_elements(base_child, extra_child);
                }
            } else {
                base.children.push(XMLNode::Element(extra_child.clone()));
            }
        }
    }
}

/// 合并 MXML 主入口，返回合并后的 XML 字符串和冲突记录
pub fn merge_mxml(
    base: Option<String>,
    extras: Vec<(String, String)>,
) -> Result<(String, Vec<(String, Vec<String>)>), MxmlError> {
    let mut path_to_extras: HashMap<String, Vec<String>> = HashMap::new();

    // 检测多模组间的路径冲突
    for (content, name) in &extras {
        if content.trim().is_empty() {
            continue;
        }
        let extra_el =
            Element::parse(content.as_bytes()).map_err(|e| MxmlError::ParseError(e.to_string()))?;

        let mut extra_paths = HashSet::new();
        collect_paths(&extra_el, "", &mut extra_paths);

        for path in extra_paths {
            path_to_extras.entry(path).or_default().push(name.clone());
        }
    }

    let mut conflicts = Vec::new();
    for (path, extra_names) in path_to_extras {
        if extra_names.len() > 1 {
            conflicts.push((path, extra_names));
        }
    }
    conflicts.sort_by(|a, b| a.0.cmp(&b.0));

    // 构建或初始化基准 XML 根节点
    let mut base_el = match base {
        Some(s) if !s.trim().is_empty() => {
            Element::parse(s.as_bytes()).map_err(|e| MxmlError::ParseError(e.to_string()))?
        }
        _ => {
            if let Some((first_extra_content, _)) = extras.first() {
                let first_el = Element::parse(first_extra_content.as_bytes())
                    .map_err(|e| MxmlError::ParseError(e.to_string()))?;
                Element {
                    name: first_el.name.clone(),
                    attributes: first_el.attributes.clone(),
                    children: Vec::new(),
                    namespace: first_el.namespace.clone(),
                    namespaces: first_el.namespaces.clone(),
                    prefix: first_el.prefix.clone(),
                }
            } else {
                Element::new("Data")
            }
        }
    };

    // 按顺序应用所有 Extra 模组的合并
    for (content, _) in &extras {
        if content.trim().is_empty() {
            continue;
        }
        let extra_el =
            Element::parse(content.as_bytes()).map_err(|e| MxmlError::ParseError(e.to_string()))?;
        merge_elements(&mut base_el, &extra_el);
    }

    let mut out_bytes = Vec::new();
    base_el
        .write(&mut out_bytes)
        .map_err(|e| MxmlError::WriteError(e.to_string()))?;

    let xml_body =
        String::from_utf8(out_bytes).map_err(|e| MxmlError::WriteError(e.to_string()))?;

    let final_xml = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{}", xml_body);

    Ok((final_xml, conflicts))
}
