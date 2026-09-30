# Tidy local workflow skills

39 workflow skills guide the installed local model. `catalog.json` is the runtime source of truth; the sibling SKILL.md files describe each workflow. These are Tidy application skills, not Codex plugins.

The model selects a workflow from the request, or the user selects one explicitly. Only that workflow is injected into investigation context. Workflow guards narrow allowed proposals, never grant execution authority. Folder names remain model-selected. Storage and History workflows hand off to their evidence-backed UI.
