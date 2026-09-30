pub mod catalog;
pub mod investigation;
pub mod planning;
pub mod prompt;
pub mod session;
pub mod store;
pub mod tools;
pub mod worker;

pub use planning::{
    ModelActionType, ModelPlanAction, ModelPlanProposal, PlanValidationError,
    build_planning_prompt, parse_and_validate_proposal, validate_model_plan,
};
pub use tools::{ReadTool, ReadToolHandler, ReadToolOutput, execute_read_tool};

#[derive(Debug, Clone)]
pub struct AgentBudget {
    pub max_steps: u8,
    pub max_context_tokens: u32,
    pub max_output_tokens: u32,
    pub timeout_seconds: u16,
}
impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_steps: 6,
            max_context_tokens: 4096,
            max_output_tokens: 384,
            timeout_seconds: 30,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStatus {
    Unavailable,
    Ready,
}
pub fn model_status() -> ModelStatus {
    ModelStatus::Ready
}

pub mod workflows;

pub mod fast_trash;

pub mod fast_extension;

pub mod intent;
