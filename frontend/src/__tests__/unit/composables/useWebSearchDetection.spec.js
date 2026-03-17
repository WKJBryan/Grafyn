import { describe, it, expect } from 'vitest'
import { detectWebSearch } from '@/composables/useWebSearchDetection'

describe('detectWebSearch', () => {
  // --- Suppression rules ---

  it('returns false for empty/short prompts', () => {
    expect(detectWebSearch('')).toMatchObject({ shouldSearch: false })
    expect(detectWebSearch('hi')).toMatchObject({ shouldSearch: false })
    expect(detectWebSearch('test me')).toMatchObject({ shouldSearch: false })
  })

  it('returns false for code-heavy prompts', () => {
    const result = detectWebSearch('```python\ndef hello(): pass\n```\nfix this code today')
    expect(result.shouldSearch).toBe(false)
  })

  it('returns false for prompts with mostly non-alpha characters', () => {
    const result = detectWebSearch('12345 67890 +-*/= {}[] <>()')
    expect(result.shouldSearch).toBe(false)
  })

  it('returns false for wikilink-heavy prompts', () => {
    const result = detectWebSearch('summarize [[My Research Note]]')
    expect(result.shouldSearch).toBe(false)
  })

  // --- Rule 1: Temporal markers ---

  it('detects current year references', () => {
    const year = new Date().getFullYear()
    const result = detectWebSearch(`What happened in ${year}?`)
    expect(result.shouldSearch).toBe(true)
    expect(result.confidence).toBe('high')
    expect(result.reason).toContain(String(year))
  })

  it('detects next year references', () => {
    const nextYear = new Date().getFullYear() + 1
    const result = detectWebSearch(`Predictions for ${nextYear}`)
    expect(result.shouldSearch).toBe(true)
    expect(result.confidence).toBe('high')
  })

  it('does not trigger on historical years', () => {
    const result = detectWebSearch('What happened in 1995 during the Renaissance?')
    expect(result.shouldSearch).toBe(false)
  })

  it('detects temporal adverbs', () => {
    expect(detectWebSearch('What is happening today in the markets')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('Tell me what is currently trending')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('What happened this week in tech')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
  })

  it('detects recency qualifiers', () => {
    expect(detectWebSearch('What is the latest version of Node.js?')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('Show me the most recent updates')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('Is this information up to date?')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
  })

  // --- Rule 2: Explicit web search intent ---

  it('detects explicit search requests', () => {
    expect(detectWebSearch('search the web for rust async patterns')).toMatchObject({
      shouldSearch: true, confidence: 'high', reason: 'Explicit search request'
    })
    expect(detectWebSearch('google the latest OpenAI pricing')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('look up online how to configure nginx')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
  })

  it('detects research directives', () => {
    expect(detectWebSearch('search for information about quantum computing')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('look up the population of Tokyo')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
  })

  it('suppresses research directives when wikilinks present', () => {
    const result = detectWebSearch('look up [[Quantum Computing Notes]] for context')
    // Wikilink suppression may or may not trigger depending on ratio —
    // but the research directive rule specifically checks for wikilinks
    expect(result.reason).not.toBe('Research query detected')
  })

  // --- Rule 3: News/event patterns ---

  it('detects news patterns', () => {
    expect(detectWebSearch('latest news about AI regulation in Europe')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('What was announced at the Apple keynote')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
    expect(detectWebSearch('weather in San Francisco this weekend')).toMatchObject({
      shouldSearch: true, confidence: 'high'
    })
  })

  // --- Rule 4: Freshness-seeking ---

  it('detects freshness-sensitive questions', () => {
    expect(detectWebSearch('how much does GPT-4 API cost per token')).toMatchObject({
      shouldSearch: true, confidence: 'medium'
    })
    expect(detectWebSearch('what is the current interest rate')).toMatchObject({
      shouldSearch: true, confidence: 'medium'
    })
    expect(detectWebSearch('who is the current president of France')).toMatchObject({
      shouldSearch: true, confidence: 'medium'
    })
  })

  it('detects ranking queries with temporal context', () => {
    const year = new Date().getFullYear()
    expect(detectWebSearch(`best laptops in ${year}`)).toMatchObject({
      shouldSearch: true
    })
    expect(detectWebSearch('top frameworks right now')).toMatchObject({
      shouldSearch: true
    })
  })

  // --- Rule 5: Comparison/market ---

  it('detects comparison queries', () => {
    expect(detectWebSearch('compare React and Angular for large apps')).toMatchObject({
      shouldSearch: true, confidence: 'medium'
    })
    expect(detectWebSearch('what are the alternatives to Slack')).toMatchObject({
      shouldSearch: true, confidence: 'medium'
    })
  })

  it('suppresses comparison when about LLM canvas responses', () => {
    expect(detectWebSearch('compare the two responses above')).toMatchObject({
      shouldSearch: false
    })
    expect(detectWebSearch('which answer is better')).toMatchObject({
      shouldSearch: false
    })
  })

  // --- Negative cases ---

  it('returns false for abstract/creative prompts', () => {
    expect(detectWebSearch('explain the concept of recursion in simple terms')).toMatchObject({
      shouldSearch: false
    })
    expect(detectWebSearch('write a poem about the ocean and its depths')).toMatchObject({
      shouldSearch: false
    })
    expect(detectWebSearch('help me brainstorm ideas for a fantasy novel')).toMatchObject({
      shouldSearch: false
    })
  })

  it('returns false for code generation prompts', () => {
    expect(detectWebSearch('write a function that sorts an array in JavaScript')).toMatchObject({
      shouldSearch: false
    })
    expect(detectWebSearch('refactor this class to use dependency injection')).toMatchObject({
      shouldSearch: false
    })
  })

  // --- Edge cases ---

  it('handles null/undefined input', () => {
    expect(detectWebSearch(null)).toMatchObject({ shouldSearch: false })
    expect(detectWebSearch(undefined)).toMatchObject({ shouldSearch: false })
  })

  it('is case-insensitive', () => {
    expect(detectWebSearch('WHAT IS THE LATEST NEWS ABOUT SPACEX')).toMatchObject({
      shouldSearch: true
    })
    expect(detectWebSearch('Search The Web For Climate Data')).toMatchObject({
      shouldSearch: true
    })
  })
})
