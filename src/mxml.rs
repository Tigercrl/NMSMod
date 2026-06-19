use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use thiserror::Error;
use xmltree::{AttributeMap, Element, EmitterConfig, XMLNode};

#[derive(Error, Debug)]
pub enum MxmlError {
    #[error("XML 解析错误: {0}")]
    ParseError(String),
    #[error("XML 序列化输出错误: {0}")]
    WriteError(String),
}

/// 仅在冲突时调用的路径组件格式化函数（避免无用分配）
fn format_element_component(el: &Element) -> String {
    let id_attr = el.attributes.get("_id").map(|s| s.as_str()).unwrap_or("");
    let name_attr = el.attributes.get("name").map(|s| s.as_str()).unwrap_or("");
    let val_attr = el.attributes.get("value").map(|s| s.as_str()).unwrap_or("");

    let val_str = if val_attr.chars().count() > 20 {
        let truncated: String = val_attr.chars().take(20).collect();
        format!("{}...", truncated)
    } else {
        val_attr.to_string()
    };

    format!("(id:{},name:{},value:{})", id_attr, name_attr, val_str)
}

/// 极致优化版：递归合并 XML 节点并检测冲突
fn merge_elements<'a>(
    base: Option<&'a Element>,
    extras: &[(&'a Element, String)],
    current_path: &mut Vec<&'a Element>, // 仅传递引用，避免每层递归分配字符串
    conflicts: &mut Vec<(String, Vec<String>)>,
) -> Element {
    // 1. 检测当前节点上的 value 属性冲突
    let b_val = base.and_then(|b| b.attributes.get("value"));
    let mut value_modifiers = Vec::new();

    for (e_el, name) in extras {
        let e_val = e_el.attributes.get("value");
        if e_val != b_val {
            value_modifiers.push(name.clone());
        }
    }

    // 只有真正冲突时，才触发高开销的路径字符串拼接
    if value_modifiers.len() > 1 {
        let path_str: String = current_path
            .iter()
            .map(|el| format_element_component(el))
            .collect::<Vec<_>>()
            .join("/");
        conflicts.push((path_str, value_modifiers));
    }

    // 2. 确定合并后节点的标签名
    let el_name = base
        .map(|b| b.name.clone())
        .or_else(|| extras.first().map(|(e, _)| e.name.clone()))
        .unwrap_or_else(|| "Property".to_string());

    // 3. 合并属性
    let mut merged_attrs = if let Some(b_el) = base {
        b_el.attributes.clone()
    } else {
        AttributeMap::new()
    };
    for (e_el, _) in extras {
        for (k, v) in &e_el.attributes {
            merged_attrs.insert(k.clone(), v.clone());
        }
    }

    let mut merged_el = Element::new(&*el_name);
    merged_el.attributes = merged_attrs;

    // 4. 使用 HashMap 建立子节点 $O(1)$ 索引，Key 使用 &str 规避内存分配
    let mut base_map = HashMap::new();
    let mut unique_keys = Vec::new();
    let mut seen_keys = HashSet::new();

    if let Some(b_el) = base {
        for child in &b_el.children {
            if let XMLNode::Element(c_el) = child {
                let name = c_el
                    .attributes
                    .get("name")
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let id = c_el.attributes.get("_id").map(|s| s.as_str()).unwrap_or("");
                let key = (name, id);
                base_map.entry(key).or_insert(c_el);
                if seen_keys.insert(key) {
                    unique_keys.push(key);
                }
            }
        }
    }

    let mut extras_maps = Vec::with_capacity(extras.len());
    for (e_el, name) in extras {
        let mut emap = HashMap::new();
        for child in &e_el.children {
            if let XMLNode::Element(c_el) = child {
                let name = c_el
                    .attributes
                    .get("name")
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let id = c_el.attributes.get("_id").map(|s| s.as_str()).unwrap_or("");
                let key = (name, id);
                emap.entry(key).or_insert(c_el);
                if seen_keys.insert(key) {
                    unique_keys.push(key);
                }
            }
        }
        extras_maps.push((emap, name));
    }

    // 5. 完美的 $O(1)$ 查找与递归合并
    for key in unique_keys {
        let b_child = base_map.get(&key).copied();

        let mut e_children = Vec::new();
        for (emap, name) in &extras_maps {
            if let Some(c_el) = emap.get(&key) {
                e_children.push((*c_el, (*name).clone()));
            }
        }

        let sample_el = b_child
            .or_else(|| e_children.first().map(|(el, _)| *el))
            .unwrap();

        // 仅压入引用
        current_path.push(sample_el);
        let merged_child = merge_elements(b_child, &e_children, current_path, conflicts);
        current_path.pop();

        merged_el.children.push(XMLNode::Element(merged_child));
    }

    merged_el
}

/// 合并 MXML 主函数
pub fn merge_mxml(
    base: Option<String>,
    extras: Vec<(String, String)>,
) -> Result<(String, Vec<(String, Vec<String>)>), MxmlError> {
    let base_doc = match base {
        Some(ref s) if !s.trim().is_empty() => {
            Some(Element::parse(Cursor::new(s)).map_err(|e| MxmlError::ParseError(e.to_string()))?)
        }
        _ => None,
    };

    let mut extra_docs = Vec::new();
    for (content, name) in extras {
        if !content.trim().is_empty() {
            let el = Element::parse(Cursor::new(content))
                .map_err(|e| MxmlError::ParseError(e.to_string()))?;
            extra_docs.push((el, name));
        }
    }

    if base_doc.is_none() && extra_docs.is_empty() {
        return Ok((
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Data />".to_string(),
            Vec::new(),
        ));
    }

    // 分配引用追踪向量
    let mut current_path = Vec::new();
    let sample_root = base_doc
        .as_ref()
        .or_else(|| extra_docs.first().map(|(el, _)| el));
    if let Some(root_el) = sample_root {
        current_path.push(root_el);
    }

    let mut conflicts = Vec::new();
    let extras_refs: Vec<(&Element, String)> = extra_docs
        .iter()
        .map(|(el, name)| (el, name.clone()))
        .collect();

    let merged_root = merge_elements(
        base_doc.as_ref(),
        &extras_refs,
        &mut current_path,
        &mut conflicts,
    );

    // 格式化输出
    let config = EmitterConfig::new()
        .perform_indent(true)
        .indent_string("  ");

    let mut buf = Vec::new();
    merged_root
        .write_with_config(&mut buf, config)
        .map_err(|e| MxmlError::WriteError(e.to_string()))?;

    let out_str = String::from_utf8(buf).map_err(|e| MxmlError::WriteError(e.to_string()))?;

    Ok((out_str, conflicts))
}
