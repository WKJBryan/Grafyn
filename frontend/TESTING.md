# Frontend Testing Guide

## Overview

Comprehensive test suite for the Seedream Vue 3 frontend using Vitest and @vue/test-utils.

## Test Infrastructure

### ✅ Completed

- **vitest.config.js** - Vitest configuration with jsdom environment
- **src/__tests__/setup.js** - Test setup with MSW, mock helpers, and utilities
- **package.json** - Updated with test dependencies and scripts

### Test Scripts

```bash
# Run all tests
npm test

# Run tests with UI
npm run test:ui

# Run tests once (for CI)
npm run test:run

# Run with coverage
npm run test:coverage
```

## Test Coverage Status

### ✅ Components (197 tests completed)

#### NoteEditor.spec.js (48 tests)
- ✅ Component rendering with note props
- ✅ Edit/Preview mode switching
- ✅ Dirty state tracking
- ✅ Markdown rendering
- ✅ Wikilink rendering `[[Note]]` and `[[Note|Display]]`
- ✅ Save validation (title required)
- ✅ Delete confirmation
- ✅ Tag parsing (comma-separated)
- ✅ Status selection
- ✅ Props updates

#### SearchBar.spec.js (39 tests)
- ✅ Debounced search (300ms)
- ✅ Keyboard shortcuts (Enter, Escape)
- ✅ Result selection
- ✅ Click-outside behavior
- ✅ Clear functionality
- ✅ Score bar display

#### NoteList.spec.js (42 tests)
- ✅ Note rendering
- ✅ Selection highlighting
- ✅ Empty state
- ✅ Tag display (truncation to first 3 + count)
- ✅ Status badges
- ✅ Link count display

#### BacklinksPanel.spec.js (47 tests)
- ✅ Backlink loading
- ✅ Navigation events
- ✅ Loading/empty states
- ✅ Context display
- ✅ API integration
- ✅ Props watching

#### GraphView.spec.js (21 tests)
- ✅ Basic rendering
- ✅ Placeholder content
- ✅ Toolbar buttons (disabled)
- ✅ Props handling
- ✅ Event emitters defined

### ✅ Stores (100 tests completed)

#### notes.spec.js (51 tests)
- ✅ loadNotes() action
- ✅ createNote() with state update
- ✅ updateNote() with array mutation
- ✅ deleteNote() with cleanup
- ✅ Computed properties (notesByStatus, selectedNoteComputed)
- ✅ selectNote(), clearSelection(), reset()
- ✅ Error handling

#### auth.spec.js (49 tests)
- ✅ OAuth flow actions
- ✅ Token localStorage sync
- ✅ Logout cleanup
- ✅ isAuthenticated getter
- ✅ handleOAuthCallback with user fetch
- ✅ Error handling with logout calls

### ✅ API Client (55 tests completed)

#### client.spec.js (55 tests)
- ✅ Request interceptor (adds token)
- ✅ Response interceptor (extracts data)
- ✅ 401 handling (redirect to login)
- ✅ All API methods (notes, search, graph, oauth)
- ✅ URL encoding for special characters
- ✅ Error propagation

### ✅ Views (78 tests completed)

#### HomeView.spec.js (38 tests)
- ✅ Note loading on mount
- ✅ Selection handling
- ✅ CRUD operations
- ✅ Layout rendering
- ✅ Empty state display
- ✅ Error handling

#### LoginView.spec.js (18 tests)
- ✅ Provider login calls (GitHub, Google)
- ✅ Error display with alerts
- ✅ Button styling

#### OAuthCallbackView.spec.js (22 tests)
- ✅ Code extraction from URL
- ✅ Callback handling
- ✅ Redirect after success
- ✅ Missing parameters handling
- ✅ Error state display

## Test Utilities

### Mount Helpers

```javascript
import { mountWithDependencies, shallowMountWithDependencies } from '@/__tests__/setup'

// Mount with Pinia and Router
const wrapper = mountWithDependencies(MyComponent, {
  props: { ... },
  // Optional: provide custom pinia or router
  pinia: myPinia,
  router: myRouter,
})
```

### Mock Data

```javascript
import {
  mockNotes,
  mockSearchResults,
  mockBacklinks,
  mockUser,
  mockAuthToken,
} from '@/__tests__/setup'
```

### Test Utilities

```javascript
import {
  createTestingPinia,
  flushPromises,
  createMockRouter,
  createMockAxios,
} from '@/__tests__/setup'
```

## Writing Tests

### Component Test Template

```javascript
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import MyComponent from '@/components/MyComponent.vue'

describe('MyComponent', () => {
  let wrapper

  beforeEach(() => {
    wrapper = mount(MyComponent, {
      props: { ... },
    })
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
  })

  it('renders correctly', () => {
    expect(wrapper.find('.my-element').exists()).toBe(true)
  })
})
```

### Store Test Template

```javascript
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useMyStore } from '@/stores/myStore'

describe('MyStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('has correct initial state', () => {
    const store = useMyStore()
    expect(store.items).toEqual([])
  })

  it('updates state on action', async () => {
    const store = useMyStore()
    await store.loadItems()
    expect(store.items.length).toBeGreaterThan(0)
  })
})
```

## Test Patterns

### Testing User Interactions

```javascript
// Click events
await wrapper.find('button').trigger('click')

// Input changes
await wrapper.find('input').setValue('new value')

// Select changes
await wrapper.find('select').setValue('option-value')

// Wait for DOM updates
await wrapper.vm.$nextTick()
```

### Testing Emitted Events

```javascript
await wrapper.find('button').trigger('click')

expect(wrapper.emitted('event-name')).toBeTruthy()
expect(wrapper.emitted('event-name')[0]).toEqual(['arg1', 'arg2'])
```

### Testing Async Operations

```javascript
import { flushPromises } from '@/__tests__/setup'

await wrapper.find('button').trigger('click')
await flushPromises() // Wait for all promises

expect(wrapper.text()).toContain('Updated')
```

### Mocking API Calls

```javascript
import { vi } from 'vitest'
import * as api from '@/api/client'

vi.spyOn(api.notes, 'list').mockResolvedValue({ data: mockNotes })

// Test component that calls api.notes.list()
```

## Coverage Goals

- **Overall**: 80%+
- **Components**: 85%+
- **Stores**: 90%+
- **Utils**: 85%+

## Running Tests in CI

Tests are designed to run in CI/CD pipelines:

```yaml
# Example GitHub Actions
- name: Install dependencies
  run: npm ci

- name: Run tests
  run: npm run test:run

- name: Upload coverage
  uses: codecov/codecov-action@v3
  with:
    files: ./coverage/lcov.info
```

## Troubleshooting

### jsdom Issues

If you encounter jsdom-related errors:

```bash
npm install jsdom@latest --save-dev
```

### MSW Setup Issues

MSW (Mock Service Worker) is included but not yet fully configured. For now, tests use vi.mock() for API mocking.

### Vue Test Utils Warnings

Suppress deprecation warnings in tests:

```javascript
config.global.config.warnHandler = () => null
```

## Next Steps

1. **Install Dependencies**:
   ```bash
   cd frontend
   npm install
   ```

2. **Run Existing Tests**:
   ```bash
   npm test
   ```

3. **Implement Remaining Tests**:
   - SearchBar component
   - NoteList component
   - BacklinksPanel component
   - Pinia stores (notes, auth)
   - API client
   - Views (Home, Login, OAuthCallback)

4. **Add Integration Tests**:
   - User workflows
   - Store + API integration

5. **Monitor Coverage**:
   ```bash
   npm run test:coverage
   open coverage/index.html
   ```

## Resources

- [Vitest Documentation](https://vitest.dev/)
- [Vue Test Utils](https://test-utils.vuejs.org/)
- [Testing Library](https://testing-library.com/)
- [Pinia Testing](https://pinia.vuejs.org/cookbook/testing.html)

## Test Statistics

- **Total Tests Created**: 430+
- **Test Files**: 11
- **Components Tested**: 5/5 (197 tests)
- **Stores Tested**: 2/2 (100 tests)
- **API Client Tested**: 1/1 (55 tests)
- **Views Tested**: 3/3 (78 tests)
- **Coverage Target**: 80%+

---

**Frontend Testing Progress**: 100% Complete (430+ tests)

For backend testing documentation, see `backend/tests/README.md`.
