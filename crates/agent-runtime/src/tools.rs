//! Bounded read-only tools for the local planning agent.
//! No write, delete, shell, or execution capability exists.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ReadTool {
    Search { query: String, limit: u16 },
    Metadata { file_id: u64 },
    TextExcerpt { file_id: u64, max_bytes: u32 },
    StorageFindings { limit: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadToolOutput {
    Success { data: String },
    Error { message: String },
}

pub trait ReadToolHandler {
    fn search(&self, query: &str, limit: u16) -> Result<String, String>;
    fn metadata(&self, file_id: u64) -> Result<String, String>;
    fn text_excerpt(&self, file_id: u64, max_bytes: u32) -> Result<String, String>;
    fn storage_findings(&self, limit: u16) -> Result<String, String>;
}

/// Dispatches a read tool request through a bounded handler.
/// Enforces limits on query lengths and result sizes.
pub fn execute_read_tool<H: ReadToolHandler>(tool: &ReadTool, handler: &H) -> ReadToolOutput {
    match tool {
        ReadTool::Search { query, limit } => {
            if query.len() > 256 {
                return ReadToolOutput::Error {
                    message: "Search query exceeds 256 bytes".into(),
                };
            }
            let bounded_limit = (*limit).clamp(1, 50);
            match handler.search(query, bounded_limit) {
                Ok(data) => ReadToolOutput::Success { data },
                Err(err) => ReadToolOutput::Error { message: err },
            }
        }
        ReadTool::Metadata { file_id } => match handler.metadata(*file_id) {
            Ok(data) => ReadToolOutput::Success { data },
            Err(err) => ReadToolOutput::Error { message: err },
        },
        ReadTool::TextExcerpt { file_id, max_bytes } => {
            let bounded_bytes = (*max_bytes).clamp(1, 4096);
            match handler.text_excerpt(*file_id, bounded_bytes) {
                Ok(data) => ReadToolOutput::Success { data },
                Err(err) => ReadToolOutput::Error { message: err },
            }
        }
        ReadTool::StorageFindings { limit } => {
            let bounded_limit = (*limit).clamp(1, 50);
            match handler.storage_findings(bounded_limit) {
                Ok(data) => ReadToolOutput::Success { data },
                Err(err) => ReadToolOutput::Error { message: err },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockHandler;
    impl ReadToolHandler for MockHandler {
        fn search(&self, query: &str, limit: u16) -> Result<String, String> {
            Ok(format!("Search results for '{query}' (limit {limit})"))
        }
        fn metadata(&self, file_id: u64) -> Result<String, String> {
            if file_id == 999 {
                Err("File not found".into())
            } else {
                Ok(format!("{{\"id\":{file_id},\"size\":1024}}"))
            }
        }
        fn text_excerpt(&self, file_id: u64, max_bytes: u32) -> Result<String, String> {
            Ok(format!(
                "Excerpt for {file_id} bounded to {max_bytes} bytes"
            ))
        }
        fn storage_findings(&self, limit: u16) -> Result<String, String> {
            Ok(format!("Storage findings (limit {limit})"))
        }
    }

    #[test]
    fn search_bounds_query_and_limit() {
        let handler = MockHandler;
        let tool = ReadTool::Search {
            query: "test".into(),
            limit: 200, // over max 50
        };
        let out = execute_read_tool(&tool, &handler);
        match out {
            ReadToolOutput::Success { data } => {
                assert!(data.contains("limit 50"));
            }
            _ => panic!("Expected success"),
        }

        let oversized_tool = ReadTool::Search {
            query: "x".repeat(300),
            limit: 10,
        };
        let out = execute_read_tool(&oversized_tool, &handler);
        match out {
            ReadToolOutput::Error { message } => {
                assert!(message.contains("exceeds 256 bytes"));
            }
            _ => panic!("Expected error for oversized query"),
        }
    }

    #[test]
    fn metadata_and_excerpt_respect_bounds() {
        let handler = MockHandler;
        let meta_tool = ReadTool::Metadata { file_id: 42 };
        let out = execute_read_tool(&meta_tool, &handler);
        match out {
            ReadToolOutput::Success { data } => assert!(data.contains("\"id\":42")),
            _ => panic!("Expected success"),
        }

        let excerpt_tool = ReadTool::TextExcerpt {
            file_id: 42,
            max_bytes: 100_000, // over max 4096
        };
        let out = execute_read_tool(&excerpt_tool, &handler);
        match out {
            ReadToolOutput::Success { data } => assert!(data.contains("bounded to 4096 bytes")),
            _ => panic!("Expected success"),
        }
    }
}
