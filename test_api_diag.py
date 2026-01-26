"""API-level diagnostic test"""
import requests
import json

base_url = 'http://127.0.0.1:8000/api/import'

# Upload
with open('test_chatgpt_data.json', 'rb') as f:
    files = {'file': ('conversations.json', f, 'application/json')}
    response = requests.post(f'{base_url}/upload', files=files)
    job_id = response.json()['id']
    print(f"Job ID: {job_id}")

# Parse
response = requests.post(f'{base_url}/{job_id}/parse')
print(f"Parse status: {response.status_code}")

# Get job directly
response = requests.get(f'{base_url}/{job_id}')
job = response.json()
print(f"\nJob state BEFORE apply:")
print(f"  status: {job['status']}")
print(f"  total_conversations: {job['total_conversations']}")
print(f"  has parsed_conversations: {job.get('parsed_conversations') is not None}")

if job.get('parsed_conversations'):
    print(f"  parsed_conversations count: {len(job['parsed_conversations'])}")
    for pc in job['parsed_conversations']:
        print(f"    - ID: {pc['id']}, Title: {pc['title']}, Messages: {len(pc['messages'])}")

# Apply
decisions = [
    {'conversation_id': c['id'], 'action': 'accept', 'distill_option': 'container_only'}
    for c in job.get('parsed_conversations', [])
]
print(f"\nDecisions to send: {json.dumps(decisions, indent=2)}")

response = requests.post(f'{base_url}/{job_id}/apply', json=decisions)
print(f"\nApply status: {response.status_code}")
print(f"Apply response: {json.dumps(response.json(), indent=2)}")

# Get job AFTER apply
response = requests.get(f'{base_url}/{job_id}')
job_after = response.json()
print(f"\nJob state AFTER apply:")
print(f"  status: {job_after['status']}")
print(f"  has decisions: {job_after.get('decisions') is not None}")
if job_after.get('decisions'):
    print(f"  decisions count: {len(job_after['decisions'])}")
