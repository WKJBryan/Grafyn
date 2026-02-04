# Security Audit Report - Grafyn Knowledge Graph Platform

**Date:** 2026-01-01  
**Auditor:** Code Simplifier Mode  
**Scope:** Backend (FastAPI) and Frontend (Vue.js)

---

## Executive Summary

This audit identified **12 critical/high-severity security vulnerabilities** and multiple code quality issues that require immediate attention. The most critical issues involve authentication bypasses, insecure token storage, and input validation gaps.

---

## Critical Security Issues (Priority 1)

### 1. Authentication Bypass in MCP OAuth
**Severity:** CRITICAL  
**Location:** [`backend/app/mcp/oauth.py:75-77`](backend/app/mcp/oauth.py:75)

**Issue:**
```python
if authorization is None:
    # Allow Claude Desktop without auth for local development
    return True
```

**Risk:** Complete authentication bypass - any request without authorization header is allowed access.

**Impact:** Unauthorized access to all protected endpoints, data exfiltration, and potential system compromise.

**Recommendation:**
- Remove development bypass in production
- Implement proper authentication for all requests
- Use environment-based configuration to control auth requirements

---

### 2. Insecure Token Storage
**Severity:** CRITICAL  
**Location:** [`backend/app/services/token_store.py`](backend/app/services/token_store.py)

**Issue:**
- OAuth tokens stored in plain text JSON file (`data/tokens.json`)
- No encryption at rest
- No file permissions set
- Predictable token IDs (`token_0`, `token_1`, etc.)

**Risk:** If attacker gains file system access, all OAuth tokens are compromised.

**Impact:** Unauthorized access to user accounts, data theft, and privilege escalation.

**Recommendation:**
- Encrypt tokens using strong encryption (AES-256-GCM)
- Use cryptographically secure random token IDs
- Set restrictive file permissions (600 on Unix)
- Consider using a proper secrets manager or database

---

### 3. Predictable Token Generation
**Severity:** HIGH  
**Location:** [`backend/app/mcp/oauth.py:62`](backend/app/mcp/oauth.py:62), [`backend/app/routers/oauth.py:72`](backend/app/routers/oauth.py:72)

**Issue:**
```python
token_id = f"token_{len(token_store._tokens)}"
```

**Risk:** Sequential, predictable token IDs allow token enumeration attacks.

**Impact:** Attackers can guess valid tokens and access other users' data.

**Recommendation:**
- Use `secrets.token_urlsafe(32)` for cryptographically secure random tokens
- Implement token expiration (e.g., 1 hour)
- Add token versioning for rotation

---

### 4. No Token Expiration
**Severity:** HIGH  
**Location:** All OAuth implementations

**Issue:** Tokens never expire, remaining valid indefinitely.

**Risk:** Compromised tokens provide permanent access until manually revoked.

**Impact:** Long-term unauthorized access even after initial compromise.

**Recommendation:**
- Implement token expiration (access tokens: 1 hour, refresh tokens: 7 days)
- Add token refresh mechanism
- Store issued_at timestamp and validate on each request

---

### 5. Missing CSRF Protection
**Severity:** HIGH  
**Location:** OAuth flows in both [`backend/app/mcp/oauth.py`](backend/app/mcp/oauth.py) and [`backend/app/routers/oauth.py`](backend/app/routers/oauth.py)

**Issue:** OAuth callback endpoints lack state parameter validation.

**Risk:** Cross-Site Request Forgery attacks can hijack OAuth flows.

**Impact:** Attackers can force users to authenticate under attacker-controlled accounts.

**Recommendation:**
- Generate cryptographically secure state parameter
- Validate state on callback
- Store state in session with short expiration

---

## High Severity Issues (Priority 2)

### 6. Path Traversal Vulnerability
**Severity:** HIGH  
**Location:** [`backend/app/services/knowledge_store.py:27-29`](backend/app/services/knowledge_store.py:27)

**Issue:**
```python
def _get_note_path(self, note_id: str) -> Path:
    return self.vault_path / f"{note_id}.md"
```

**Risk:** If note_id contains `../`, attacker can access files outside vault directory.

**Impact:** Unauthorized file access, potential system file exposure.

**Recommendation:**
- Sanitize note_id to remove path traversal sequences
- Validate resolved path is within vault_path
- Use `Path.resolve().is_relative_to()` for validation

---

### 7. Insecure CORS Configuration
**Severity:** HIGH  
**Location:** [`backend/app/main.py:52-58`](backend/app/main.py:52)

**Issue:**
```python
allow_origins=settings.cors_origins,
allow_credentials=True,
allow_methods=["*"],
allow_headers=["*"],
```

**Risk:** Overly permissive CORS with credentials enabled.

**Impact:** Cross-origin attacks, credential theft.

**Recommendation:**
- Restrict to specific origins only
- Disable credentials if not required
- Validate Origin header explicitly
- Use environment-specific CORS policies

---

### 8. Missing Rate Limiting on Sensitive Endpoints
**Severity:** HIGH  
**Location:** All routers except health check

**Issue:** Only health check has rate limiting (`@limiter.limit("10 per minute")`).

**Risk:** Brute force attacks, DoS vulnerabilities, credential stuffing.

**Impact:** Service disruption, account enumeration, resource exhaustion.

**Recommendation:**
- Apply rate limiting to all endpoints
- Use stricter limits for auth endpoints (5/minute)
- Implement IP-based and user-based limiting
- Add CAPTCHA for repeated failures

---

### 9. Insecure Frontend Token Storage
**Severity:** HIGH  
**Location:** [`frontend/src/api/client.js:14`](frontend/src/api/client.js:14), [`frontend/src/stores/auth.js:8`](frontend/src/stores/auth.js:8)

**Issue:** Authentication tokens stored in `localStorage`.

**Risk:** XSS attacks can steal tokens and impersonate users.

**Impact:** Complete session hijacking, unauthorized access.

**Recommendation:**
- Use httpOnly, secure, SameSite cookies
- Implement short-lived tokens with refresh mechanism
- Add Content Security Policy (CSP)
- Implement XSS protection headers

---

## Medium Severity Issues (Priority 3)

### 10. Insufficient Input Validation
**Severity:** MEDIUM  
**Location:** Multiple endpoints

**Issues:**
- Note titles/content not validated for malicious content
- Search queries only length-limited, not sanitized
- No validation of file uploads (if any)

**Risk:** XSS, injection attacks, data corruption.

**Impact:** Data integrity issues, client-side attacks.

**Recommendation:**
- Implement comprehensive input validation
- Sanitize user content before storage
- Use parameterized queries (if using DB)
- Add output encoding

---

### 11. Sensitive Data in Logs
**Severity:** MEDIUM  
**Location:** [`backend/app/middleware/logging.py`](backend/app/middleware/logging.py)

**Issue:** Logs all requests without filtering sensitive data.

**Risk:** Tokens, passwords, or sensitive data may be logged.

**Impact:** Credential leakage through logs.

**Recommendation:**
- Redact Authorization headers
- Filter sensitive request parameters
- Implement log sanitization
- Use structured logging with levels

---

### 12. No HTTPS Enforcement
**Severity:** MEDIUM  
**Location:** Frontend and backend configuration

**Issue:** No HTTPS enforcement or HSTS headers.

**Risk:** Man-in-the-middle attacks, credential interception.

**Impact:** Data interception, session hijacking.

**Recommendation:**
- Enforce HTTPS in production
- Add HSTS header
- Use secure cookie flags
- Implement certificate pinning for mobile

---

## Code Quality & Simplification Issues

### 1. Code Duplication
**Severity:** MEDIUM  
**Locations:**
- OAuth implementation duplicated between [`mcp/oauth.py`](backend/app/mcp/oauth.py) and [`routers/oauth.py`](backend/app/routers/oauth.py)
- Service initialization pattern repeated in all routers

**Impact:** Maintenance burden, inconsistent security fixes.

**Recommendation:**
- Extract common OAuth logic to shared service
- Use FastAPI dependency injection for services
- Implement factory pattern for service initialization

---

### 2. Global State Management
**Severity:** MEDIUM  
**Location:** All routers use global variables for services

**Issue:**
```python
knowledge_store = None  # Global in each router
```

**Impact:** Testing difficulties, race conditions, state leaks.

**Recommendation:**
- Use FastAPI dependency injection
- Pass services via app.state
- Remove global variables

---

### 3. Inconsistent Error Handling
**Severity:** LOW  
**Location:** Multiple files

**Issue:** Mix of HTTPException, generic exceptions, and no error handling.

**Impact:** Poor user experience, information leakage.

**Recommendation:**
- Implement centralized error handler
- Create custom exception classes
- Sanitize error messages
- Log errors securely

---

### 4. Missing Security Headers
**Severity:** LOW  
**Location:** [`backend/app/main.py`](backend/app/main.py)

**Issue:** No security headers (X-Frame-Options, X-Content-Type-Options, etc.)

**Impact:** Clickjacking, MIME sniffing attacks.

**Recommendation:**
- Add security middleware
- Implement OWASP recommended headers
- Use helmet-like middleware for FastAPI

---

## Recommended Implementation Order

### Phase 1: Critical Fixes (Immediate)
1. Remove authentication bypass in MCP OAuth
2. Implement secure token storage with encryption
3. Use cryptographically secure token generation
4. Add token expiration

### Phase 2: High Priority (1-2 weeks)
5. Implement CSRF protection
6. Fix path traversal vulnerability
7. Secure CORS configuration
8. Add comprehensive rate limiting
9. Switch to httpOnly cookies for frontend

### Phase 3: Medium Priority (2-4 weeks)
10. Implement input validation
11. Sanitize logging
12. Enforce HTTPS
13. Add security headers

### Phase 4: Code Quality (Ongoing)
14. Refactor duplicate code
15. Implement dependency injection
16. Centralize error handling
17. Add security tests

---

## Additional Recommendations

### Security Testing
- Implement automated security scanning (SAST, DAST)
- Add dependency vulnerability scanning
- Perform penetration testing
- Implement security unit tests

### Monitoring & Alerting
- Add security event logging
- Implement anomaly detection
- Set up alerts for suspicious activities
- Monitor authentication failures

### Documentation
- Document security architecture
- Create security guidelines for developers
- Document threat model
- Maintain security changelog

### Compliance
- Review OWASP Top 10 compliance
- Consider GDPR implications (data storage)
- Implement data retention policies
- Add privacy controls

---

## Conclusion

The Grafyn platform has several critical security vulnerabilities that require immediate attention. The authentication bypass and insecure token storage are particularly concerning and should be addressed before any production deployment.

The codebase would also benefit significantly from refactoring to eliminate duplication and improve maintainability, which will make implementing security fixes easier and more consistent.

**Estimated Effort:**
- Critical fixes: 2-3 days
- High priority: 1-2 weeks
- Medium priority: 2-4 weeks
- Code quality: Ongoing

**Overall Security Rating: ⚠️ NEEDS IMPROVEMENT**
