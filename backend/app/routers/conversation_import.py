"""LLM import router for conversation import"""

import logging
from typing import List, Optional

from fastapi import APIRouter, UploadFile, File, Request, HTTPException, Depends
from fastapi.responses import FileResponse
from pydantic import BaseModel, Field

from app.models.import_models import (
    ImportDecision,
    ImportJob,
    ImportSummary,
    PreviewResult,
    SummarizationSettings,
)
from app.services.import_service import ImportService
from app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()

router = APIRouter(tags=["import"])


class AssessRequest(BaseModel):
    """Request body for quality assessment endpoint"""
    summarization_settings: Optional[SummarizationSettings] = None


def get_import_service(request: Request) -> ImportService:
    """Get import service instance from app state."""
    return request.app.state.import_service


@router.post("/upload", response_model=ImportJob)
async def upload_file(
    file: UploadFile = File(...), service: ImportService = Depends(get_import_service)
):
    """
    Upload LLM export file and create import job.

    Supported formats: ChatGPT (conversations.json), Claude (.dms/.json), Grok (.json), Gemini (.json)
    """
    try:
        # Read file content
        file_content = await file.read()

        # Create import job
        job = await service.upload_file(file_content, file.filename)

        return job
    except Exception as e:
        logger.error(f"File upload failed: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/{job_id}", response_model=ImportJob)
async def get_job(job_id: str, service: ImportService = Depends(get_import_service)):
    """Get import job status and details."""
    try:
        job = service.jobs.get(job_id)
        if not job:
            raise HTTPException(status_code=404, detail=f"Job not found: {job_id}")
        return job
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Failed to get job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.post("/{job_id}/parse", response_model=ImportJob)
async def parse_file(job_id: str, service: ImportService = Depends(get_import_service)):
    """Parse uploaded file and extract conversations."""
    try:
        job = await service.parse_file(job_id)
        return job
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Parse failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/{job_id}/preview", response_model=PreviewResult)
async def get_preview(
    job_id: str, service: ImportService = Depends(get_import_service)
):
    """
    Get preview of parsed conversations.

    Returns conversations, duplicates found, and estimated notes to create.
    """
    try:
        preview = await service.get_preview(job_id)
        return preview
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Preview failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.post("/{job_id}/assess", response_model=ImportJob)
async def assess_quality(
    job_id: str,
    body: Optional[AssessRequest] = None,
    service: ImportService = Depends(get_import_service),
):
    """
    Assess conversation quality using LLM.
    
    Uses the selected OpenRouter model to:
    - Score conversations for relevance and informativeness
    - Generate detailed summaries
    - Suggest import action (import_and_distill, import_only, skip)
    - Identify key topics for knowledge linking
    - Find related notes in the existing knowledge base
    
    Args:
        job_id: Import job ID
        body: Optional settings (model_id, detail_level, etc.)
    
    Returns:
        Updated job with quality assessments on each conversation
    """
    try:
        settings = body.summarization_settings if body else None
        job = await service.assess_conversations_quality(job_id, settings)
        return job
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Assessment failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))

@router.post("/{job_id}/apply", response_model=ImportSummary)
async def apply_import(
    job_id: str,
    decisions: List[ImportDecision],
    service: ImportService = Depends(get_import_service),
):
    """
    Apply import with user decisions.

    Each decision specifies:
    - action: "accept" | "skip" | "merge" | "modify"
    - target_note_id: For merge action, existing note to merge into
    - distill_option: "container_only" | "auto_distill" | "custom"
    - custom_atoms: For custom distillation, list of atomic note templates
    """
    try:
        summary = await service.apply_import(job_id, decisions)
        return summary
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Import failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.delete("/{job_id}")
async def cancel_job(job_id: str, service: ImportService = Depends(get_import_service)):
    """Cancel and cleanup import job."""
    try:
        success = service.cancel_job(job_id)
        if not success:
            raise HTTPException(status_code=404, detail=f"Job not found: {job_id}")
        return {"message": f"Job {job_id} cancelled"}
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Cancel failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.post("/{job_id}/revert")
async def revert_import(
    job_id: str, 
    request: Request,
    service: ImportService = Depends(get_import_service)
):
    """
    Revert the last import by deleting all notes created during that import.
    """
    try:
        result = await service.revert_import(job_id, request.app.state.knowledge_store)
        return result
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Revert failed for job {job_id}: {e}")
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/")
async def list_jobs(service: ImportService = Depends(get_import_service)):
    """List all import jobs."""
    return list(service.jobs.values())
