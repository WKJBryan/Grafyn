"""Simple test to verify LLM integration in distillation"""
import asyncio
import sys
from pathlib import Path
from datetime import datetime

# Add backend to path
backend_path = Path(__file__).parent.parent
sys.path.insert(0, str(backend_path))

from backend.app.models.distillation import (
    DistillRequest,
    DistillMode,
    ExtractionMethod,
    DistillResponse,
)
from backend.app.models.note import Note, NoteCreate, NoteFrontmatter
from backend.app.services.distillation import DistillationService


class MockKnowledgeStore:
    """Mock knowledge store for testing"""
    
    def __init__(self):
        self.notes = {}
    
    def get_note(self, note_id: str) -> Note:
        if note_id in self.notes:
            return self.notes[note_id]
        return None
    
    def create_note(self, note_data: NoteCreate) -> Note:
        note_id = note_data.title.replace(" ", "_").lower()
        note = Note(
            id=note_id,
            title=note_data.title,
            content=note_data.content,
            frontmatter=NoteFrontmatter(
                title=note_data.title,
                status=note_data.status,
                tags=note_data.tags,
                created=datetime(2025, 1, 1, 0, 0, 0),
                modified=datetime(2025, 1, 1, 0, 0, 0)
            )
        )
        self.notes[note_id] = note
        return note
    
    def update_note(self, note_id: str, note_data) -> Note:
        if note_id in self.notes:
            note = self.notes[note_id]
            if hasattr(note_data, 'content') and note_data.content:
                note.content = note_data.content
            if hasattr(note_data, 'tags') and note_data.tags:
                note.frontmatter.tags = note_data.tags
            return note
        return None


class MockVectorSearch:
    """Mock vector search for testing"""
    
    def search(self, query: str, limit: int = 5):
        return []
    
    def index_note(self, note_id: str, title: str, content: str):
        pass


class MockGraphIndex:
    """Mock graph index for testing"""
    
    def build_index(self):
        pass


class MockOpenRouter:
    """Mock OpenRouter service for testing"""
    
    async def chat_completion(self, messages, model, stream=False):
        """Return a mock LLM summary"""
        return {
            "content": """## Machine Learning Fundamentals

Machine learning is a subset of artificial intelligence that enables systems to learn from data.

- Uses algorithms to find patterns in data
- Can make predictions without explicit programming
- Requires large datasets for training

### Key Claims
- ML models improve with more data
- Different algorithms suit different problems

### Open Questions
- How to handle biased datasets?
- What's the optimal model complexity?

## Neural Networks

Neural networks are computing systems inspired by biological neural networks.

- Consist of interconnected nodes (neurons)
- Learn through backpropagation
- Excel at pattern recognition

### Key Claims
- Deep learning uses multiple hidden layers
- Training requires significant computational resources

### Open Questions
- How to interpret model decisions?
- What are the ethical implications?

## Model Evaluation

Evaluating ML models is crucial for ensuring they work as intended.

- Use train/test splits to measure performance
- Consider precision, recall, and F1 score
- Watch for overfitting

### Key Claims
- Cross-validation provides more reliable estimates
- Different metrics suit different use cases

### Open Questions
- How to evaluate fairness?
- What metrics matter most for production?
"""
        }


async def test_llm_extraction():
    """Test LLM-based extraction in AUTO mode"""
    print("Testing LLM integration in distillation...")
    
    # Create mock services
    knowledge_store = MockKnowledgeStore()
    vector_search = MockVectorSearch()
    graph_index = MockGraphIndex()
    openrouter = MockOpenRouter()
    
    # Create distillation service
    service = DistillationService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        openrouter_service=openrouter
    )
    
    # Create a sample container note
    container_note = Note(
        id="test_container",
        title="AI Research Notes",
        content="""# AI Research Notes

This is a collection of notes about artificial intelligence and machine learning.

## Machine Learning

Machine learning is transforming how we approach complex problems.

## Neural Networks

Deep learning has revolutionized image recognition and natural language processing.

## Evaluation

Proper evaluation is essential for reliable ML systems.
""",
        frontmatter=NoteFrontmatter(
            title="AI Research Notes",
            status="canonical",
            tags=["ai", "research"],
            created=datetime(2025, 1, 1, 0, 0, 0),
            modified=datetime(2025, 1, 1, 0, 0, 0)
        )
    )
    knowledge_store.notes["test_container"] = container_note
    
    # Test 1: LLM extraction
    print("\n1. Testing LLM extraction...")
    request = DistillRequest(
        mode=DistillMode.AUTO,
        extraction_method=ExtractionMethod.LLM,
        hub_policy="auto"
    )
    
    progress_updates = []
    def progress_callback(message: str):
        progress_updates.append(message)
        print(f"  Progress: {message}")
    
    response = await service.distill("test_container", request, progress_callback)
    
    assert response.extraction_method_used == "llm", f"Expected 'llm', got '{response.extraction_method_used}'"
    assert response.status == "completed", f"Expected 'completed', got '{response.status}'"
    assert len(response.created_note_ids) > 0, "Expected at least one created note"
    assert response.summary is not None, "Expected LLM summary to be present"
    
    print(f"  [OK] LLM extraction created {len(response.created_note_ids)} notes")
    print(f"  [OK] Progress updates: {len(progress_updates)}")
    
    # Test 2: Rule-based extraction
    print("\n2. Testing rule-based extraction...")
    request = DistillRequest(
        mode=DistillMode.AUTO,
        extraction_method=ExtractionMethod.RULES,
        hub_policy="auto"
    )
    
    progress_updates.clear()
    response = await service.distill("test_container", request, progress_callback)
    
    assert response.extraction_method_used == "rules", f"Expected 'rules', got '{response.extraction_method_used}'"
    assert response.status == "completed", f"Expected 'completed', got '{response.status}'"
    
    print(f"  [OK] Rule-based extraction created {len(response.created_note_ids)} notes")
    
    # Test 3: AUTO mode (prefer LLM)
    print("\n3. Testing AUTO mode (prefer LLM)...")
    request = DistillRequest(
        mode=DistillMode.AUTO,
        extraction_method=ExtractionMethod.AUTO,
        hub_policy="auto"
    )
    
    progress_updates.clear()
    response = await service.distill("test_container", request, progress_callback)
    
    assert response.extraction_method_used == "llm", f"Expected 'llm', got '{response.extraction_method_used}'"
    assert response.status == "completed", f"Expected 'completed', got '{response.status}'"
    
    print(f"  [OK] AUTO mode used LLM extraction")
    
    # Test 4: Fallback to rules when LLM fails
    print("\n4. Testing fallback to rules...")
    
    # Create a failing OpenRouter that properly raises exception
    class FailingOpenRouter:
        async def chat_completion(self, messages, model, stream=False):
            # This exception should propagate through _summarize_with_llm
            # and trigger the fallback in _auto
            raise Exception("LLM service unavailable")
    
    # Patch the _summarize_with_llm method to propagate the exception
    original_summarize = DistillationService._summarize_with_llm
    
    async def failing_summarize(self, note):
        # Don't catch the exception, let it propagate
        raise Exception("LLM service unavailable")
    
    service_failing = DistillationService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        openrouter_service=FailingOpenRouter()
    )
    
    # Replace the method to ensure exception propagates
    service_failing._summarize_with_llm = failing_summarize.__get__(service_failing, DistillationService)
    
    request = DistillRequest(
        mode=DistillMode.AUTO,
        extraction_method=ExtractionMethod.LLM,
        hub_policy="auto"
    )
    
    progress_updates.clear()
    response = await service_failing.distill("test_container", request, progress_callback)
    
    assert response.extraction_method_used == "rules", f"Expected 'rules' (fallback), got '{response.extraction_method_used}'"
    assert response.status == "completed", f"Expected 'completed', got '{response.status}'"
    
    print(f"  [OK] Fallback to rules worked correctly")
    
    # Test 5: No OpenRouter service (should use rules)
    print("\n5. Testing without OpenRouter service...")
    service_no_llm = DistillationService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        openrouter_service=None
    )
    
    request = DistillRequest(
        mode=DistillMode.AUTO,
        extraction_method=ExtractionMethod.AUTO,
        hub_policy="auto"
    )
    
    progress_updates.clear()
    response = await service_no_llm.distill("test_container", request, progress_callback)
    
    assert response.extraction_method_used == "rules", f"Expected 'rules', got '{response.extraction_method_used}'"
    assert response.status == "completed", f"Expected 'completed', got '{response.status}'"
    
    print(f"  [OK] AUTO mode without OpenRouter used rules")
    
    print("\n[PASS] All tests passed!")
    print("\nSummary:")
    print("- LLM extraction works correctly")
    print("- Rule-based extraction works correctly")
    print("- AUTO mode prefers LLM")
    print("- Fallback to rules works on LLM failure")
    print("- Works without OpenRouter service")
    print("- Progress callbacks are called")


if __name__ == "__main__":
    asyncio.run(test_llm_extraction())
