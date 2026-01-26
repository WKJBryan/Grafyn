"""Direct unit test of ImportService.apply_import"""
import asyncio
import sys
import os

# Add project root to path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from backend.app.services.import_service import ImportService
from backend.app.services.knowledge_store import KnowledgeStore
from backend.app.models.import_models import ImportDecision, ParsedConversation, ParsedMessage, ConversationMetadata
from datetime import datetime, timezone

async def test_apply_import():
    # Create minimal services
    knowledge_store = KnowledgeStore()
    import_service = ImportService(knowledge_store=knowledge_store)
    
    # Create a test conversation manually
    conv = ParsedConversation(
        id="test-conv-001",
        title="Test Conversation",
        platform="chatgpt",
        messages=[
            ParsedMessage(
                index=0,
                role="user",
                content="Hello, how are you?",
                timestamp=datetime.now(timezone.utc)
            ),
            ParsedMessage(
                index=1,
                role="assistant", 
                content="I'm doing great, thanks for asking!",
                timestamp=datetime.now(timezone.utc)
            )
        ],
        metadata=ConversationMetadata(
            platform="chatgpt",
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc),
            message_count=2,
            model_info=["gpt-4"]
        ),
        suggested_tags=["test", "chatgpt"]
    )
    
    # Simulate a job with parsed conversations
    from backend.app.models.import_models import ImportJob
    job = ImportJob(
        id="test-job-001",
        status="parsed",
        file_path="/tmp/test.json",
        file_name="test.json",
        platform="chatgpt",
        total_conversations=1,
        parsed_conversations=[conv],
        created_at=datetime.now(timezone.utc),
        updated_at=datetime.now(timezone.utc)
    )
    
    # Add to import service
    import_service.jobs["test-job-001"] = job
    
    # Verify jobs state
    print(f"Job in service: {import_service.jobs.get('test-job-001') is not None}")
    print(f"Parsed conversations: {len(job.parsed_conversations)}")
    
    # Create decision
    decisions = [
        ImportDecision(
            conversation_id="test-conv-001",
            action="accept",
            distill_option="container_only"
        )
    ]
    
    # Apply import
    print("\nCalling apply_import...")
    try:
        result = await import_service.apply_import("test-job-001", decisions)
        print(f"\nResult:")
        print(f"  Imported: {result.imported}")
        print(f"  Container notes: {result.container_notes}")
        print(f"  Notes created: {result.notes_created}")
        print(f"  Errors: {result.errors}")
        
        if result.imported > 0:
            print("\n✅ TEST PASSED!")
            return True
        else:
            print("\n❌ TEST FAILED - No notes imported")
            return False
    except Exception as e:
        print(f"Error: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return False

if __name__ == "__main__":
    success = asyncio.run(test_apply_import())
    sys.exit(0 if success else 1)
