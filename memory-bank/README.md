# Seedream Memory Bank

> **Purpose:** Central repository for project knowledge, decisions, and practical guidance
> **Last Updated:** 2025-12-31
> **Status:** Active

## Overview

The Memory Bank complements the technical documentation in the [`docs/`](../docs/) directory by capturing:

- **Project Context**: History, evolution, and milestones
- **Architecture Decisions**: Why technical choices were made (ADRs)
- **Development Patterns**: Coding standards and best practices
- **Known Issues**: Common problems and their solutions
- **Configuration**: Setup and environment details
- **Quick Reference**: Concise lookup guides
- **Workflows**: Standardized procedures

## Structure

```
memory-bank/
├── README.md                          # This file - index and overview
├── 01-project-context/                 # Project history and background
├── 02-architecture-decisions/         # Architecture Decision Records (ADRs)
├── 03-development-patterns/           # Coding standards and conventions
├── 04-known-issues/                   # Issues and solutions
├── 05-configuration/                 # Setup and environment reference
├── 06-quick-reference/               # Concise lookup guides
└── 07-workflows/                     # Development and deployment procedures
```

## Quick Links

### For New Developers
- [Project History](./01-project-context/history.md) - Understand the project's origins
- [Setup Guide](./05-configuration/setup-guide.md) - Get started quickly
- [Development Workflow](./07-workflows/development-workflow.md) - Daily development process

### For Architecture Understanding
- [ADR Index](./02-architecture-decisions/README.md) - All architecture decisions
- [Technology Stack](./02-architecture-decisions/adr-001-technology-stack.md) - Why we chose these technologies
- [Architecture Pattern](./02-architecture-decisions/adr-002-architecture-pattern.md) - Service layer design

### For Development
- [Coding Standards](./03-development-patterns/coding-standards.md) - Style guides
- [Backend Patterns](./03-development-patterns/backend-patterns.md) - Python/FastAPI patterns
- [Frontend Patterns](./03-development-patterns/frontend-patterns.md) - Vue 3 patterns

### For Troubleshooting
- [Known Issues](./04-known-issues/) - Common problems and solutions
- [Troubleshooting Guide](./05-configuration/troubleshooting.md) - Setup issues

### For Quick Reference
- [API Endpoints](./06-quick-reference/api-endpoints.md) - Concise API reference
- [Data Models](./06-quick-reference/data-models.md) - Data model cheat sheet
- [Services](./06-quick-reference/services.md) - Service layer reference

### For Workflows
- [Development Workflow](./07-workflows/development-workflow.md) - Daily development process
- [Contribution Workflow](./07-workflows/contribution-workflow.md) - How to contribute
- [Deployment Workflow](./07-workflows/deployment-workflow.md) - Deployment procedures

## Relationship to Technical Documentation

| Memory Bank | Technical Docs | Purpose |
|-------------|----------------|---------|
| Memory Bank | [`docs/`](../docs/) | Complementary |
| Project context, decisions, patterns | Architecture, API contracts, data models | Why vs What |
| Practical guidance | Technical specifications | How vs What |
| Evolution and history | Current state | Past vs Present |

## When to Use Memory Bank

**Use Memory Bank when you need to:**
- Understand why a technical decision was made
- Find solutions to known issues
- Follow development patterns and conventions
- Learn about project history and evolution
- Get quick reference information
- Follow development or deployment workflows

**Use Technical Docs when you need to:**
- Understand the current architecture
- Look up API specifications
- Review data models and schemas
- Follow development setup instructions

## Maintaining the Memory Bank

### Adding New ADRs
1. Create new ADR in [`02-architecture-decisions/`](./02-architecture-decisions/)
2. Update the ADR index
3. Follow the ADR template

### Documenting Issues
1. Add to appropriate file in [`04-known-issues/`](./04-known-issues/)
2. Include problem description, root cause, and solution
3. Reference related ADRs if applicable

### Updating Patterns
1. Modify relevant pattern files in [`03-development-patterns/`](./03-development-patterns/)
2. Ensure examples are current
3. Update related quick references

### Review Schedule

- **Quarterly**: Review all active ADRs
- **On Change**: Create new ADR when decisions change
- **On Request**: Review ADRs when questions arise
- **Periodically**: Update patterns and known issues

### ADR Quality Criteria

A good ADR should:

- ✅ Clearly state the problem being solved
- ✅ Explain the decision and its rationale
- ✅ List alternatives considered
- ✅ Describe consequences (positive and negative)
- ✅ Provide references for more context
- ✅ Be concise and focused

## Related Resources

- [Technical Documentation](../docs/)
- [Project README](../README.md)
- [GitHub Repository](https://github.com/your-org/seedream)

---

**Note:** The Memory Bank is a living document. It evolves with the project and should be updated as new decisions are made, patterns emerge, or issues are discovered.
