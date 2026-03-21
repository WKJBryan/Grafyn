import DOMPurify from 'dompurify'
import { marked } from 'marked'

/**
 * Render markdown to sanitized HTML.
 * All user-controlled markdown MUST go through this function before v-html binding.
 */
export function renderMarkdown(content) {
  if (!content) return ''
  marked.setOptions({ breaks: true, gfm: true })
  return DOMPurify.sanitize(marked(content))
}
