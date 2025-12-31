# Contribution Workflow

> **Purpose:** Guide for contributing to OrgAI project
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document guides contributors through the process of contributing to OrgAI project.

## Getting Started

### First-Time Contributors

1. **Fork Repository**
   ```bash
   # Visit: https://github.com/your-org/orgai
   # Click "Fork" button
   ```

2. **Clone Your Fork**
   ```bash
   git clone https://github.com/your-username/orgai.git
   cd orgai
   ```

3. **Add Upstream Remote**
   ```bash
   git remote add upstream https://github.com/your-org/orgai.git
   ```

4. **Set Up Development Environment**
   - Follow [Setup Guide](../05-configuration/setup-guide.md)
   - Install dependencies
   - Configure environment variables

### Existing Contributors

```bash
# Pull latest changes
git checkout main
git pull upstream main

# Create feature branch
git checkout -b feature/your-feature-name
```

## Contribution Types

### Feature Development

**When to use:**
- Adding new functionality
- Enhancing existing features
- Non-breaking changes

**Branch naming:**
```
feature/<feature-name>
```

**Example:**
```bash
git checkout -b feature/add-note-templates
```

### Bug Fixes

**When to use:**
- Fixing reported bugs
- Correcting errors
- Non-breaking fixes

**Branch naming:**
```
fix/<issue-number-or-description>
```

**Example:**
```bash
git checkout -b fix/42-wikilink-parsing
```

### Documentation

**When to use:**
- Updating README
- Adding code comments
- Improving inline docs

**Branch naming:**
```
docs/<documentation-area>
```

**Example:**
```bash
git checkout -b docs/api-endpoints
```

### Refactoring

**When to use:**
- Code cleanup
- Performance improvements
- Non-functional changes

**Branch naming:**
```
refactor/<component-or-area>
```

**Example:**
```bash
git checkout -b refactor/service-layer
```

## Development Process

### 1. Make Changes

```bash
# Edit files
# Follow coding standards
# Add tests for new code
# Update documentation
```

### 2. Test Changes

#### Backend Tests

```bash
# Run unit tests
cd backend
pytest tests/unit/ -v

# Run integration tests
pytest tests/integration/ -v

# Run with coverage
pytest --cov=app --cov-report=html
```

#### Frontend Tests

```bash
# Run component tests
cd frontend
npm test

# Run with coverage
npm run test:coverage
```

### 3. Commit Changes

```bash
# Stage changes
git add .

# Review changes
git status
git diff --staged

# Commit with conventional commits
git commit -m "feat: add new feature

- Implement feature X
- Add tests for feature
- Update documentation"
```

### 4. Sync with Upstream

```bash
# Fetch upstream changes
git fetch upstream

# Rebase your branch
git rebase upstream/main

# Resolve conflicts if any
# Continue rebase
git rebase --continue
```

### 5. Push to Your Fork

```bash
# Push to your fork
git push origin feature/your-feature-name
```

## Pull Request Process

### 1. Create Pull Request

```bash
# Using GitHub CLI
gh pr create \
  --title "Add new feature" \
  --body "Description of changes..." \
  --base main \
  --head feature/your-feature-name

# Or visit GitHub web interface
# https://github.com/your-org/orgai/compare/main...feature/your-feature-name
```

### 2. PR Description Template

```markdown
## Description
Brief description of changes.

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] All tests pass locally
- [ ] Manual testing completed

## Checklist
- [ ] Code follows project standards
- [ ] Self-review completed
- [ ] No new warnings generated
- [ ] Documentation updated
- [ ] CHANGELOG updated (if applicable)
- [ ] No merge conflicts

## Related Issues
Closes #123
Related to #456
```

### 3. PR Review Process

#### For Contributors

- **Be responsive** to review feedback
- **Make requested changes** promptly
- **Ask questions** if feedback is unclear
- **Update tests** if new edge cases found

#### For Reviewers

- **Review thoroughly** for:
  - Code quality
  - Test coverage
  - Security issues
  - Performance impact
  - Documentation completeness

- **Provide constructive feedback**:
  - Be specific about issues
  - Suggest improvements
  - Ask questions if unclear
  - Approve with conditions if needed

### 4. Addressing Review Feedback

```bash
# Make changes based on feedback
# ...

# Commit changes
git commit -m "fix: address review feedback

- Fix issue X
- Improve code quality
- Add missing tests"

# Push to branch
git push origin feature/your-feature-name

# PR will update automatically
```

### 5. Merging

#### Automatic Merge

If PR passes all checks:
- Maintainer merges PR
- Branch is deleted
- Changes are in main

#### Manual Merge

If merge conflicts:
- Resolve conflicts locally
- Push resolved changes
- Update PR

## Code Review Guidelines

### What to Review

1. **Functionality**
   - Does the code work as intended?
   - Are edge cases handled?
   - Is error handling appropriate?

2. **Code Quality**
   - Is code readable?
   - Are naming conventions followed?
   - Is there unnecessary complexity?
   - Are there code smells?

3. **Testing**
   - Are tests comprehensive?
   - Do tests cover edge cases?
   - Is test coverage adequate?
   - Are tests well-written?

4. **Documentation**
   - Is code documented?
   - Are docstrings complete?
   - Is README updated if needed?
   - Are examples clear?

5. **Security**
   - Are inputs validated?
   - Are there security vulnerabilities?
   - Is sensitive data handled properly?
   - Are dependencies up to date?

6. **Performance**
   - Is code efficient?
   - Are there performance issues?
   - Are resources managed properly?
   - Is caching used appropriately?

### Review Etiquette

**Do:**
- ✅ Be constructive and specific
- ✅ Provide examples for improvements
- ✅ Ask questions if something is unclear
- ✅ Acknowledge good work
- ✅ Respond in a timely manner

**Don't:**
- ❌ Be vague or dismissive
- ❌ Make personal attacks
- ❌ Demand changes without explanation
- ❌ Ignore review requests
- ❌ Delay responses unnecessarily

## Issue Reporting

### Bug Reports

**Template:**
```markdown
## Bug Description
Brief description of the bug.

## Steps to Reproduce
1. Go to '...'
2. Click on '....'
3. Scroll to '....'
4. See error

## Expected Behavior
What should happen.

## Actual Behavior
What actually happens.

## Environment
- OS: [e.g., Windows 10, macOS 14, Ubuntu 22.04]
- Browser: [e.g., Chrome 120, Firefox 121]
- Version: [e.g., v0.2.0]

## Screenshots
If applicable, add screenshots.

## Additional Context
Any other relevant information.
```

### Feature Requests

**Template:**
```markdown
## Feature Description
Clear description of the requested feature.

## Problem Statement
What problem does this solve?

## Proposed Solution
How should this feature work?

## Alternatives Considered
What other approaches did you consider?

## Additional Context
Any other relevant information.
```

## Recognition

### Contributor Recognition

Contributors are recognized in:
- README contributors section
- Release notes
- Project documentation

### Ways to Contribute

1. **Code**: Write features, fix bugs
2. **Documentation**: Improve docs, add examples
3. **Tests**: Write tests, improve coverage
4. **Reviews**: Review pull requests
5. **Issues**: Report bugs, suggest features
6. **Community**: Help others in discussions

## Guidelines

### Code of Conduct

**Be Respectful:**
- Treat everyone with respect
- Be inclusive and welcoming
- Focus on what is best for the community
- Accept constructive criticism gracefully

**Be Professional:**
- Keep discussions focused
- Avoid personal attacks
- Respect differing viewpoints
- Communicate clearly and professionally

### License

By contributing, you agree that your contributions will be licensed under the same license as the project.

## Getting Help

### Where to Ask

1. **GitHub Discussions**: For questions and general discussion
2. **GitHub Issues**: For bug reports and feature requests
3. **Documentation**: Check existing docs first
4. **Code Review**: Ask in PR comments

### Before Asking

- [ ] Read the documentation
- [ ] Search existing issues
- [ ] Try to solve it yourself
- [ ] Provide context when asking

## Related Documentation

- [Development Workflow](./development-workflow.md)
- [Coding Standards](../03-development-patterns/coding-standards.md)
- [Testing Patterns](../03-development-patterns/testing-patterns.md)

---

**See Also:**
- [Project Overview](../../docs/project-overview.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)
- [Development Guide - Frontend](../../docs/development-guide-frontend.md)
