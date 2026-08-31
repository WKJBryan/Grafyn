import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it } from 'vitest'
import CompanionShell from '@/components/companion/CompanionShell.vue'

async function mountShell(path = '/') {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: { template: '<div />' } },
      { path: '/recall', component: { template: '<div />' } },
      { path: '/twin', component: { template: '<div />' } },
      { path: '/canvas', component: { template: '<div />' } },
      { path: '/unavailable', name: 'capability-unavailable', component: { template: '<div />' } },
    ],
  })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(CompanionShell, {
    slots: { default: '<main data-test="shell-content">Content</main>' },
    global: { plugins: [router] },
  })
  await flushPromises()
  return wrapper
}

describe('CompanionShell', () => {
  it('renders the compact content and exact four primary destinations', async () => {
    const wrapper = await mountShell('/recall')

    expect(wrapper.get('[data-test="shell-content"]').text()).toBe('Content')
    const links = wrapper.findAll('.companion-nav-link')
    expect(links.map(link => link.text())).toEqual(['Capture', 'Recall', 'Twin', 'Canvas'])
    expect(links.map(link => link.attributes('href'))).toEqual(['/', '/recall', '/twin', '/canvas'])
    expect(links.map(link => link.attributes('aria-current'))).toEqual([
      undefined,
      'page',
      undefined,
      undefined,
    ])
  })

  it('provides an accessible label for the primary navigation', async () => {
    const wrapper = await mountShell()

    expect(wrapper.get('nav').attributes('aria-label')).toBe('Primary')
  })

  it('keeps the blocked destination current on the unavailable route', async () => {
    const wrapper = await mountShell('/unavailable?capability=recall')
    const links = wrapper.findAll('.companion-nav-link')

    expect(links[1].attributes('aria-current')).toBe('page')
    expect(links.filter(link => link.attributes('aria-current') === 'page')).toHaveLength(1)
  })
})
