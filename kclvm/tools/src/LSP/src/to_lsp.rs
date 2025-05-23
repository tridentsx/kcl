use im_rc::HashMap;
use kclvm_error::Diagnostic as KCLDiagnostic;
use kclvm_error::DiagnosticId;
use kclvm_error::Level;
use kclvm_error::Message;
use kclvm_error::Position as KCLPos;
use kclvm_utils::path::PathPrefix;
use lsp_types::*;
use serde_json::json;

use std::{
    path::{Component, Path, Prefix},
    str::FromStr,
};

/// Convert pos format to lsp position.
/// The position in lsp protocol is different with position in ast node whose line number is 1 based.
pub fn lsp_pos(pos: &KCLPos) -> Position {
    Position {
        line: pos.line.checked_sub(1).unwrap_or(0) as u32,
        character: pos.column.unwrap_or(0) as u32,
    }
}

/// Convert start and pos format to lsp location.
/// The position of the location in lsp protocol is different with position in ast node whose line number is 1 based.
pub fn lsp_location(file_path: String, start: &KCLPos, end: &KCLPos) -> Option<Location> {
    let uri = Url::from_file_path(file_path).ok()?;
    Some(Location {
        uri,
        range: Range {
            start: lsp_pos(start),
            end: lsp_pos(end),
        },
    })
}

/// Convert KCL message to the LSP diagnostic.
pub fn kcl_msg_to_lsp_diags(
    msg: &Message,
    severity: DiagnosticSeverity,
    related_msg: Vec<Message>,
    code: Option<NumberOrString>,
) -> Diagnostic {
    let range = msg.range.clone();
    let start_position = lsp_pos(&range.0);
    let end_position = lsp_pos(&range.1);

    let data = msg
        .suggested_replacement
        .as_ref()
        .map(|s_vec| {
            s_vec
                .iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<&String>>()
        })
        .map(|s| {
            if s.is_empty() {
                json!({ "suggested_replacement": "" })
            } else {
                json!({ "suggested_replacement": s })
            }
        });

    let related_information = if related_msg.is_empty() {
        None
    } else {
        Some(
            related_msg
                .iter()
                .filter_map(|m| match Url::from_file_path(m.range.0.filename.clone()) {
                    Ok(uri) => Some(DiagnosticRelatedInformation {
                        location: Location {
                            uri,
                            range: Range {
                                start: lsp_pos(&m.range.0),
                                end: lsp_pos(&m.range.1),
                            },
                        },
                        message: m.message.clone(),
                    }),
                    Err(_) => None,
                })
                .collect(),
        )
    };

    Diagnostic {
        range: Range::new(start_position, end_position),
        severity: Some(severity),
        code,
        code_description: None,
        source: None,
        message: msg.message.clone(),
        related_information,
        tags: None,
        data,
    }
}

/// Convert KCL error level to the LSP diagnostic severity.
pub fn kcl_err_level_to_severity(level: Level) -> DiagnosticSeverity {
    match level {
        Level::Error => DiagnosticSeverity::ERROR,
        Level::Warning => DiagnosticSeverity::WARNING,
        Level::Note => DiagnosticSeverity::HINT,
        Level::Suggestions => DiagnosticSeverity::HINT,
    }
}

/// Convert KCL Diagnostic to LSP Diagnostics.
pub fn kcl_diag_to_lsp_diags(diag: &KCLDiagnostic) -> HashMap<String, Vec<Diagnostic>> {
    let mut diags_map: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    for (idx, msg) in diag.messages.iter().enumerate() {
        let filename = msg.range.0.filename.clone();

        let mut related_msg = diag.messages.clone();
        related_msg.remove(idx);
        let code = if diag.code.is_some() {
            Some(kcl_diag_id_to_lsp_diag_code(diag.code.clone().unwrap()))
        } else {
            None
        };

        let lsp_diag = kcl_msg_to_lsp_diags(
            msg,
            kcl_err_level_to_severity(diag.level),
            related_msg,
            code,
        );

        diags_map.entry(filename).or_insert(vec![]).push(lsp_diag);
    }
    diags_map
}

/// Convert KCL Diagnostic to LSP Diagnostics.
#[allow(unused)]
pub(crate) fn kcl_diag_to_lsp_diags_by_file(
    diag: &KCLDiagnostic,
    file_name: &str,
) -> Vec<Diagnostic> {
    let mut diags = vec![];
    for (idx, msg) in diag.messages.iter().enumerate() {
        if msg.range.0.filename.adjust_canonicalization() == file_name.adjust_canonicalization() {
            let mut related_msg = diag.messages.clone();
            related_msg.remove(idx);
            let code = if diag.code.is_some() {
                Some(kcl_diag_id_to_lsp_diag_code(diag.code.clone().unwrap()))
            } else {
                None
            };

            let lsp_diag = kcl_msg_to_lsp_diags(
                msg,
                kcl_err_level_to_severity(diag.level),
                related_msg,
                code,
            );

            diags.push(lsp_diag);
        }
    }
    diags
}

/// Convert KCL Diagnostic ID to LSP Diagnostics code.
/// Todo: use unique id/code instead of name()
pub(crate) fn kcl_diag_id_to_lsp_diag_code(id: DiagnosticId) -> NumberOrString {
    match id {
        DiagnosticId::Error(err) => NumberOrString::String(err.name()),
        DiagnosticId::Warning(warn) => NumberOrString::String(warn.name()),
        DiagnosticId::Suggestions => NumberOrString::String("suggestion".to_string()),
    }
}

pub(crate) fn url_from_path(path: impl AsRef<Path>) -> anyhow::Result<Url> {
    url_from_path_with_drive_lowercasing(path)
}

/// Returns a `Url` object from a given path, will lowercase drive letters if present.
/// This will only happen when processing Windows paths.
///
/// When processing non-windows path, this is essentially do the same as `Url::from_file_path`.
pub(crate) fn url_from_path_with_drive_lowercasing(path: impl AsRef<Path>) -> anyhow::Result<Url> {
    let component_has_windows_drive = path.as_ref().components().any(|comp| {
        if let Component::Prefix(c) = comp {
            match c.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => return true,
                _ => return false,
            }
        }
        false
    });

    // VSCode expects drive letters to be lowercased, whereas rust will uppercase the drive letters.
    if component_has_windows_drive {
        let url_original = Url::from_file_path(&path).map_err(|_| {
            anyhow::anyhow!("can't convert path to url: {}", path.as_ref().display())
        })?;

        let drive_partition: Vec<&str> = url_original.as_str().rsplitn(2, ':').collect();

        // There is a drive partition, but we never found a colon.
        // This should not happen, but in this case we just pass it through.
        if drive_partition.len() == 1 {
            return Ok(url_original);
        }

        let joined = drive_partition[1].to_ascii_lowercase() + ":" + drive_partition[0];
        let url = Url::from_str(&joined)
            .map_err(|e| anyhow::anyhow!("Url from str ParseError: {}", e))?;
        Ok(url)
    } else {
        Ok(Url::from_file_path(&path).map_err(|_| {
            anyhow::anyhow!("can't convert path to url: {}", path.as_ref().display())
        })?)
    }
}
