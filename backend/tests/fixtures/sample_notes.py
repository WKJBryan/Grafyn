"""Sample note data for testing"""
from datetime import datetime
from typing import List, Dict


def get_sample_note(**kwargs) -> Dict:
    """
    Get a sample note with default values that can be overridden.

    Args:
        **kwargs: Override any default values

    Returns:
        Dictionary with note data
    """
    defaults = {
        "title": "Sample Note",
        "content": "This is a sample note for testing.",
        "status": "draft",
        "tags": ["test"],
    }
    defaults.update(kwargs)
    return defaults


def get_notes_with_wikilinks() -> List[Dict]:
    """
    Get a set of interconnected notes with wikilinks for graph testing.

    Returns a list of notes that link to each other in a known pattern:
    - A links to B and C
    - B links to C and D
    - C links to D
    - D has no outgoing links
    - E is isolated (no links)
    """
    return [
        {
            "title": "Note A",
            "content": """# Note A

This is the first note in the chain.

It links to [[Note B]] and [[Note C|Note C with display text]].

## Additional Content

Some more content to make it realistic.
""",
            "status": "canonical",
            "tags": ["chain", "first"],
        },
        {
            "title": "Note B",
            "content": """# Note B

This is the second note.

It references [[Note C]] and [[Note D]].

Also mentions [[Note A]] in the content.
""",
            "status": "canonical",
            "tags": ["chain", "middle"],
        },
        {
            "title": "Note C",
            "content": """# Note C

Third note in the sequence.

Links to [[Note D]].

Has backlinks from [[Note A]] and [[Note B]].
""",
            "status": "evidence",
            "tags": ["chain", "middle"],
        },
        {
            "title": "Note D",
            "content": """# Note D

Final note in the chain.

No outgoing links, but has backlinks from multiple notes.
""",
            "status": "draft",
            "tags": ["chain", "last"],
        },
        {
            "title": "Note E",
            "content": """# Note E

This note is isolated.

It has no wikilinks to other notes and should not appear in the graph traversal.
""",
            "status": "draft",
            "tags": ["isolated"],
        },
    ]


def get_notes_for_search_testing() -> List[Dict]:
    """
    Get notes with specific content for semantic search testing.

    Returns notes with varying content about different topics
    to test search relevance and ranking.
    """
    return [
        {
            "title": "Python Programming Basics",
            "content": """# Python Programming Basics

Python is a high-level, interpreted programming language known for its simplicity and readability.

## Key Features
- Easy to learn syntax
- Dynamic typing
- Extensive standard library
- Great for beginners

Python is widely used in web development, data science, machine learning, and automation.
""",
            "status": "canonical",
            "tags": ["python", "programming", "tutorial"],
        },
        {
            "title": "JavaScript Fundamentals",
            "content": """# JavaScript Fundamentals

JavaScript is the programming language of the web, running in browsers and on servers via Node.js.

## Core Concepts
- Event-driven programming
- Asynchronous execution
- Prototypal inheritance
- First-class functions

Essential for modern web development and full-stack applications.
""",
            "status": "canonical",
            "tags": ["javascript", "programming", "web"],
        },
        {
            "title": "Machine Learning Introduction",
            "content": """# Machine Learning Introduction

Machine learning is a subset of artificial intelligence that enables systems to learn from data.

## Types of ML
- Supervised learning
- Unsupervised learning
- Reinforcement learning

Python is the most popular language for machine learning, with libraries like TensorFlow and PyTorch.
""",
            "status": "evidence",
            "tags": ["ml", "ai", "python", "data-science"],
        },
        {
            "title": "Database Design Principles",
            "content": """# Database Design Principles

Good database design is crucial for application performance and maintainability.

## Key Principles
- Normalization
- Indexing strategies
- Query optimization
- Data integrity

Relational databases use SQL, while NoSQL databases offer flexibility for unstructured data.
""",
            "status": "canonical",
            "tags": ["database", "sql", "architecture"],
        },
        {
            "title": "Recipe: Chocolate Chip Cookies",
            "content": """# Chocolate Chip Cookies

A delicious homemade chocolate chip cookie recipe.

## Ingredients
- 2 cups flour
- 1 cup butter
- 1 cup sugar
- 2 eggs
- 2 cups chocolate chips

## Instructions
1. Preheat oven to 350°F
2. Mix butter and sugar
3. Add eggs and flour
4. Fold in chocolate chips
5. Bake for 12 minutes

Enjoy warm with milk!
""",
            "status": "draft",
            "tags": ["recipe", "baking", "dessert"],
        },
    ]


def get_notes_with_special_characters() -> List[Dict]:
    """
    Get notes with special characters and unicode.

    Returns notes that test handling of:
    - Unicode characters
    - Special markdown syntax
    - Special characters in wikilinks

    NOTE: Does NOT include edge cases like very long titles or empty content
    to avoid Windows MAX_PATH issues. Use get_edge_case_notes() for those.
    """
    return [
        {
            "title": "Unicode Test Chinese",
            "content": """# Unicode Content

This note contains various unicode characters:

- Chinese: 你好世界
- Japanese: こんにちは世界
- Arabic: مرحبا بالعالم
- Emoji: 🎉 🚀 💻 🌟

Links to [[Note with Emojis]].
""",
            "status": "draft",
            "tags": ["unicode", "test", "i18n"],
        },
        {
            "title": "Note with Emojis",
            "content": """# Emoji Test 🎨

Testing emoji in titles and content.

Symbols: ℃ ℉ © ® ™ € £ ¥

Math: ∑ ∏ ∫ √ ∞ ≈ ≠ ≤ ≥
""",
            "status": "draft",
            "tags": ["emoji", "symbols"],
        },
        {
            "title": "Special Characters Test",
            "content": """# Testing Special Characters

Title has special characters that need proper handling.

Content with code: `const x = {key: "value"};`

Wikilink with spaces: [[Note A]].
""",
            "status": "draft",
            "tags": ["special-chars", "test"],
        },
    ]


def get_edge_case_notes() -> List[Dict]:
    """
    Get notes for edge case testing.

    NOTE: These may fail on Windows due to MAX_PATH limits.
    """
    return [
        {
            "title": "A" * 50,  # Moderately long title (safe for Windows)
            "content": "This note has a long title to test title length handling.",
            "status": "draft",
            "tags": ["edge-case", "long-title"],
        },
        {
            "title": "Empty Content Note",
            "content": "",
            "status": "draft",
            "tags": ["edge-case", "empty"],
        },
    ]


def get_large_note_content() -> str:
    """
    Generate a large note content for performance testing.

    Returns a markdown document with ~10,000 words.
    """
    paragraphs = []

    for i in range(200):
        paragraphs.append(f"""## Section {i + 1}

This is section {i + 1} of a large document. It contains multiple sentences
to simulate a real-world long-form document. Performance testing requires
realistic data sizes to identify bottlenecks.

The content includes various markdown elements like **bold text**, *italic text*,
and `code snippets`. It also includes lists:

- Item 1
- Item 2
- Item 3

And links to [[Other Notes]] and [[Related Content]].
""")

    return "\n\n".join(paragraphs)
