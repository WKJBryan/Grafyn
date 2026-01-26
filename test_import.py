"""Complete test - all steps in quick succession without delays"""
import requests
import json
import sys

base_url = 'http://127.0.0.1:8000/api/import'

def test_import():
    # Upload
    with open('test_chatgpt_data.json', 'rb') as f:
        files = {'file': ('conversations.json', f, 'application/json')}
        response = requests.post(f'{base_url}/upload', files=files)
        if response.status_code != 200:
            print(f"Upload failed: {response.text}")
            return False
        job_id = response.json()['id']
        print(f"Upload OK - Job: {job_id}")
    
    # Parse (immediately)
    response = requests.post(f'{base_url}/{job_id}/parse')
    if response.status_code != 200:
        print(f"Parse failed: {response.text}")
        return False
    parsed = response.json()
    print(f"Parse OK - Platform: {parsed['platform']}, Conversations: {parsed['total_conversations']}")
    
    # Get preview (immediately)
    response = requests.get(f'{base_url}/{job_id}/preview')
    if response.status_code != 200:
        print(f"Preview failed: {response.text}")
        return False
    preview = response.json()
    print(f"Preview OK - {len(preview['conversations'])} conversations")
    
    # Apply (immediately)
    decisions = [
        {'conversation_id': c['id'], 'action': 'accept', 'distill_option': 'container_only'}
        for c in preview['conversations']
    ]
    response = requests.post(f'{base_url}/{job_id}/apply', json=decisions)
    if response.status_code != 200:
        print(f"Apply failed: {response.text}")
        return False
    result = response.json()
    
    print(f"\n=== IMPORT RESULT ===")
    print(f"Imported: {result['imported']}")
    print(f"Skipped: {result['skipped']}")
    print(f"Failed: {result['failed']}")
    print(f"Notes created: {result['notes_created']}")
    
    if result['imported'] > 0:
        print("\n✅ TEST PASSED: Notes were imported successfully!")
        return True
    else:
        print("\n❌ TEST FAILED: No notes were imported")
        # Show detailed errors if any
        if result.get('errors'):
            print(f"Errors: {json.dumps(result['errors'], indent=2)}")
        return False

if __name__ == "__main__":
    success = test_import()
    sys.exit(0 if success else 1)
