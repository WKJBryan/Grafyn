/**
 * Smart web search detection for canvas prompts.
 *
 * Analyzes prompt text to determine whether web search should be
 * automatically enabled. Returns a detection result with reason text
 * shown to the user in the PromptDialog.
 */

const CURRENT_YEAR = new Date().getFullYear()

// Years that indicate time-sensitive queries (last year through 5 years ahead)
const YEAR_REGEX = new RegExp(`\\b(${CURRENT_YEAR - 1}|${Array.from({ length: 6 }, (_, i) => CURRENT_YEAR + i).join('|')})\\b`)

// Rule 1: Temporal markers
const TEMPORAL_ADVERBS = /\b(today|yesterday|right now|currently|this week|this month|this year|tonight|this morning)\b/i
const RECENCY_QUALIFIERS = /\b(latest|newest|most recent|up[- ]to[- ]date|just released|just launched|just announced)\b/i

// Rule 2: Explicit web search intent
const EXPLICIT_SEARCH = /\b(search the web|search online|web search|google|look up online|find online)\b/i
const RESEARCH_DIRECTIVES = /\b(search for|look up|find out)\b/i

// Rule 3: News/event patterns
const NEWS_PATTERNS = /\b(news about|latest news|breaking news?|announced|election results?|stock price|weather in|forecast)\b/i

// Rule 4: Information-seeking with freshness need
const FRESHNESS_SEEKING = /\b(what is the current|who is the current|how much does .{1,30} cost|price of|pricing for)\b/i
const BEST_TOP_PATTERN = /\b(best|top)\b.{0,30}\b(right now|this year|in \d{4})\b/i

// Rule 5: Comparison/market
const COMPARISON_TERMS = /\b(compare|versus|\bvs\b|alternatives to|competitors)\b/i
const COMPARISON_SUPPRESSION = /\b(response|answer|output|result|above|below)\b/i

// Suppression patterns
const WIKILINK_PATTERN = /\[\[.+?\]\]/
const CODE_BLOCK_START = /^```/

/**
 * Analyze prompt text for web search need.
 * @param {string} prompt
 * @returns {{ shouldSearch: boolean, reason: string|null, confidence: 'high'|'medium'|null }}
 */
export function detectWebSearch(prompt) {
  const NO_SEARCH = { shouldSearch: false, reason: null, confidence: null }

  if (!prompt || prompt.trim().length < 10) return NO_SEARCH

  const text = prompt.trim()

  // Suppression: code-heavy prompts
  if (CODE_BLOCK_START.test(text)) return NO_SEARCH
  const alphaChars = text.replace(/[^a-zA-Z]/g, '').length
  if (alphaChars / text.length < 0.5) return NO_SEARCH

  // Suppression: wikilink-heavy prompts (user referencing vault notes)
  const wikilinkCount = (text.match(/\[\[.+?\]\]/g) || []).length
  const wordCount = text.split(/\s+/).length
  if (wikilinkCount > 0 && wikilinkCount >= wordCount / 3) return NO_SEARCH

  // Rule 1: Temporal markers (high confidence)
  if (YEAR_REGEX.test(text)) {
    const yearMatch = text.match(YEAR_REGEX)
    return { shouldSearch: true, reason: `Time-sensitive query (${yearMatch[0]})`, confidence: 'high' }
  }
  if (TEMPORAL_ADVERBS.test(text)) {
    return { shouldSearch: true, reason: 'Time-sensitive query detected', confidence: 'high' }
  }
  if (RECENCY_QUALIFIERS.test(text)) {
    return { shouldSearch: true, reason: 'Recency-sensitive query detected', confidence: 'high' }
  }

  // Rule 2: Explicit web search intent (high confidence)
  if (EXPLICIT_SEARCH.test(text)) {
    return { shouldSearch: true, reason: 'Explicit search request', confidence: 'high' }
  }
  if (RESEARCH_DIRECTIVES.test(text) && !WIKILINK_PATTERN.test(text)) {
    return { shouldSearch: true, reason: 'Research query detected', confidence: 'high' }
  }

  // Rule 3: News/event patterns (high confidence)
  if (NEWS_PATTERNS.test(text)) {
    return { shouldSearch: true, reason: 'Current events query detected', confidence: 'high' }
  }

  // Rule 4: Information-seeking with freshness need (medium confidence)
  if (FRESHNESS_SEEKING.test(text)) {
    return { shouldSearch: true, reason: 'Freshness-sensitive query detected', confidence: 'medium' }
  }
  if (BEST_TOP_PATTERN.test(text)) {
    return { shouldSearch: true, reason: 'Ranking query detected', confidence: 'medium' }
  }

  // Rule 5: Comparison/market (medium confidence) — suppress if about LLM responses
  if (COMPARISON_TERMS.test(text) && !COMPARISON_SUPPRESSION.test(text)) {
    return { shouldSearch: true, reason: 'Comparison query detected', confidence: 'medium' }
  }

  return NO_SEARCH
}
