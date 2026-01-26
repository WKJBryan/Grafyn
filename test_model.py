"""Direct test of ImportDecision parsing"""
import requests
import json
from pydantic import BaseModel
from typing import List, Optional, Dict, Any, Literal

class ImportDecision(BaseModel):
    conversation_id: str
    action: Literal["accept", "skip", "merge", "modify"]
    target_note_id: Optional[str] = None
    modifications: Optional[Dict[str, Any]] = None
    distill_option: Literal["container_only", "auto_distill", "custom"] = "auto_distill"

# Test local parsing
decisions_dict = [
    {
        'conversation_id': 'test-conversation-001',
        'action': 'accept',
        'distill_option': 'container_only'
    }
]

for d in decisions_dict:
    decision = ImportDecision(**d)
    print(f"conversation_id: {decision.conversation_id}")
    print(f"action: {decision.action}")
    print(f"distill_option: {decision.distill_option}")
    print(f"Action is 'accept': {decision.action == 'accept'}")
