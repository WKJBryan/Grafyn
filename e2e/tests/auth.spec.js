/**
 * E2E Tests: Authentication Flow
 *
 * Tests OAuth login and session management
 * Note: Full OAuth flow requires mocking or test accounts
 */

import { test, expect } from '@playwright/test'

test.describe('Authentication', () => {
  test.describe('Login Page', () => {
    test('should display login page', async ({ page }) => {
      await page.goto('/login')

      await expect(page.locator('.login-view, .login-container')).toBeVisible()
    })

    test('should display welcome message', async ({ page }) => {
      await page.goto('/login')

      await expect(page.locator('.login-title, h1')).toContainText('Grafyn')
    })

    test('should display GitHub login button', async ({ page }) => {
      await page.goto('/login')

      await expect(page.locator('button:has-text("GitHub")')).toBeVisible()
    })

    test('should display Google login button', async ({ page }) => {
      await page.goto('/login')

      await expect(page.locator('button:has-text("Google")')).toBeVisible()
    })

    test('should have styled login buttons', async ({ page }) => {
      await page.goto('/login')

      const githubBtn = page.locator('button:has-text("GitHub")')
      const googleBtn = page.locator('button:has-text("Google")')

      await expect(githubBtn).toHaveClass(/btn-primary/)
      await expect(googleBtn).toHaveClass(/btn-secondary/)
    })
  })

  test.describe('OAuth Flow', () => {
    test('should initiate GitHub OAuth on button click', async ({ page }) => {
      await page.goto('/login')

      // Set up request interception
      const [request] = await Promise.all([
        page.waitForRequest(req => req.url().includes('/api/oauth/authorize/github')),
        page.click('button:has-text("GitHub")'),
      ])

      expect(request.url()).toContain('/api/oauth/authorize/github')
    })

    test('should initiate Google OAuth on button click', async ({ page }) => {
      await page.goto('/login')

      const [request] = await Promise.all([
        page.waitForRequest(req => req.url().includes('/api/oauth/authorize/google')),
        page.click('button:has-text("Google")'),
      ])

      expect(request.url()).toContain('/api/oauth/authorize/google')
    })
  })

  test.describe('OAuth Callback', () => {
    test('should display loading state during callback', async ({ page }) => {
      // Mock the callback endpoint
      await page.route('**/api/oauth/callback/**', route => {
        // Delay response to see loading state
        setTimeout(() => {
          route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ access_token: 'test-token' }),
          })
        }, 1000)
      })

      await page.route('**/api/oauth/user', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ name: 'Test User', email: 'test@example.com' }),
        })
      })

      await page.goto('/oauth/callback/github?code=test-code')

      await expect(page.locator('.spinner, .loading')).toBeVisible()
      await expect(page.locator('text=Completing sign in')).toBeVisible()
    })

    test('should show error for invalid callback', async ({ page }) => {
      await page.route('**/api/oauth/callback/**', route => {
        route.fulfill({
          status: 400,
          contentType: 'application/json',
          body: JSON.stringify({ detail: 'Invalid authorization code' }),
        })
      })

      await page.goto('/oauth/callback/github?code=invalid-code')

      await expect(page.locator('text=Authentication failed')).toBeVisible()
    })

    test('should show error when missing code parameter', async ({ page }) => {
      await page.goto('/oauth/callback/github')

      await expect(page.locator('text=Invalid OAuth callback parameters')).toBeVisible()
    })

    test('should show retry button on error', async ({ page }) => {
      await page.goto('/oauth/callback/github')

      await expect(page.locator('button:has-text("Try Again")')).toBeVisible()
    })

    test('should navigate to login on retry click', async ({ page }) => {
      await page.goto('/oauth/callback/github')

      await page.click('button:has-text("Try Again")')

      await expect(page).toHaveURL('/login')
    })

    test('should redirect to home after successful login', async ({ page }) => {
      // Mock successful OAuth callback
      await page.route('**/api/oauth/callback/**', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ access_token: 'test-token' }),
        })
      })

      await page.route('**/api/oauth/user', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ name: 'Test User', email: 'test@example.com' }),
        })
      })

      await page.goto('/oauth/callback/github?code=valid-code')

      await expect(page).toHaveURL('/')
    })

    test('should redirect to custom URL after login', async ({ page }) => {
      await page.route('**/api/oauth/callback/**', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ access_token: 'test-token' }),
        })
      })

      await page.route('**/api/oauth/user', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ name: 'Test User' }),
        })
      })

      await page.goto('/oauth/callback/github?code=valid-code&redirect=/notes')

      await expect(page).toHaveURL('/notes')
    })
  })

  test.describe('Session Management', () => {
    test('should store token in localStorage', async ({ page }) => {
      await page.route('**/api/oauth/callback/**', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ access_token: 'stored-test-token' }),
        })
      })

      await page.route('**/api/oauth/user', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ name: 'Test User' }),
        })
      })

      await page.goto('/oauth/callback/github?code=valid-code')
      await page.waitForURL('/')

      const token = await page.evaluate(() => localStorage.getItem('auth_token'))
      expect(token).toBe('stored-test-token')
    })

    test('should handle 401 by redirecting to login', async ({ page }) => {
      // Set up a token
      await page.goto('/')
      await page.evaluate(() => localStorage.setItem('auth_token', 'expired-token'))

      // Mock 401 response
      await page.route('**/api/notes', route => {
        route.fulfill({
          status: 401,
          contentType: 'application/json',
          body: JSON.stringify({ detail: 'Unauthorized' }),
        })
      })

      await page.reload()

      await expect(page).toHaveURL('/login')
    })

    test('should clear token on logout', async ({ page }) => {
      // Set up initial state
      await page.goto('/')
      await page.evaluate(() => localStorage.setItem('auth_token', 'test-token'))

      await page.route('**/api/oauth/logout', route => {
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({}),
        })
      })

      // Find and click logout if available
      const logoutBtn = page.locator('button:has-text("Logout"), a:has-text("Logout")')
      if (await logoutBtn.isVisible()) {
        await logoutBtn.click()

        const token = await page.evaluate(() => localStorage.getItem('auth_token'))
        expect(token).toBeNull()
      }
    })
  })
})
