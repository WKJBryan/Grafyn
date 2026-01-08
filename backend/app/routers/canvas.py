"""Canvas API router for Multi-LLM canvas sessions"""
import asyncio
import json
import logging
from typing import List

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import StreamingResponse

from backend.app.models.canvas import (
    CanvasSession,
    CanvasSessionListItem,
    CanvasCreate,
    CanvasUpdate,
    PromptRequest,
    DebateStartRequest,
    DebateContinueRequest,
    ModelInfo,
    TilePositionUpdate,
    CanvasViewport,
    DebateMode,
)
from backend.app.services.canvas_store import CanvasSessionStore
from backend.app.services.openrouter import OpenRouterService
from backend.app.middleware.rate_limit import limiter

logger = logging.getLogger(__name__)
router = APIRouter()


def get_canvas_store(request: Request) -> CanvasSessionStore:
    """Get canvas store from app state"""
    return request.app.state.canvas_store


def get_openrouter(request: Request) -> OpenRouterService:
    """Get OpenRouter service from app state"""
    return request.app.state.openrouter


# --- Session Management ---


@router.get("", response_model=List[CanvasSessionListItem])
async def list_sessions(request: Request):
    """List all canvas sessions"""
    store = get_canvas_store(request)
    return store.list_sessions()


@router.post("", response_model=CanvasSession, status_code=201)
async def create_session(data: CanvasCreate, request: Request):
    """Create a new canvas session"""
    store = get_canvas_store(request)
    return store.create_session(data)


@router.get("/models/available", response_model=List[ModelInfo])
@limiter.limit("30 per minute")
async def list_available_models(request: Request):
    """Get list of available models from OpenRouter"""
    openrouter = get_openrouter(request)

    if not openrouter.is_configured():
        raise HTTPException(
            status_code=503, detail="OpenRouter API key not configured"
        )

    models = await openrouter.list_models()
    return [
        ModelInfo(
            id=m["id"],
            name=m["name"],
            provider=m["provider"],
            context_length=m["context_length"],
            pricing=m["pricing"],
            supports_streaming=m["supports_streaming"],
        )
        for m in models
    ]


@router.get("/{session_id}", response_model=CanvasSession)
async def get_session(session_id: str, request: Request):
    """Get a specific canvas session"""
    store = get_canvas_store(request)
    session = store.get_session(session_id)
    if not session:
        raise HTTPException(status_code=404, detail="Session not found")
    return session


@router.put("/{session_id}", response_model=CanvasSession)
async def update_session(session_id: str, data: CanvasUpdate, request: Request):
    """Update canvas session metadata"""
    store = get_canvas_store(request)
    session = store.update_session(session_id, data)
    if not session:
        raise HTTPException(status_code=404, detail="Session not found")
    return session


@router.delete("/{session_id}", status_code=204)
async def delete_session(session_id: str, request: Request):
    """Delete a canvas session"""
    store = get_canvas_store(request)
    if not store.delete_session(session_id):
        raise HTTPException(status_code=404, detail="Session not found")


# --- Viewport ---


@router.put("/{session_id}/viewport")
async def update_viewport(
    session_id: str, viewport: CanvasViewport, request: Request
):
    """Update canvas viewport state"""
    store = get_canvas_store(request)
    if not store.update_viewport(session_id, viewport):
        raise HTTPException(status_code=404, detail="Session not found")
    return {"status": "updated"}


# --- Tile Position ---


@router.put("/{session_id}/tiles/{tile_id}/position")
async def update_tile_position(
    session_id: str, tile_id: str, position: TilePositionUpdate, request: Request
):
    """Update a tile's position on the canvas"""
    store = get_canvas_store(request)
    if not store.update_tile_position(session_id, tile_id, position):
        raise HTTPException(status_code=404, detail="Tile not found")
    return {"status": "updated"}


# --- Prompt & Streaming ---


@router.post("/{session_id}/prompt")
@limiter.limit("20 per minute")
async def send_prompt(
    session_id: str, prompt_request: PromptRequest, request: Request
):
    """Send a prompt to multiple models with SSE streaming"""
    store = get_canvas_store(request)
    openrouter = get_openrouter(request)

    if not openrouter.is_configured():
        raise HTTPException(
            status_code=503, detail="OpenRouter API key not configured"
        )

    session = store.get_session(session_id)
    if not session:
        raise HTTPException(status_code=404, detail="Session not found")

    # Create the prompt tile
    tile = store.add_prompt_tile(
        session_id,
        prompt_request.prompt,
        prompt_request.models,
        prompt_request.system_prompt,
    )

    if not tile:
        raise HTTPException(status_code=500, detail="Failed to create tile")

    async def event_generator():
        """Generate SSE events for all model streams"""
        # Build messages
        messages = []
        if prompt_request.system_prompt:
            messages.append({"role": "system", "content": prompt_request.system_prompt})
        messages.append({"role": "user", "content": prompt_request.prompt})

        # Track state for each model
        model_content = {m: "" for m in prompt_request.models}
        model_done = set()

        # Queue for multiplexing streams
        queue = asyncio.Queue()

        async def stream_model(model_id: str):
            """Process stream for a single model"""
            try:
                async for chunk in openrouter.stream_completion(
                    model_id,
                    messages,
                    prompt_request.temperature,
                    prompt_request.max_tokens,
                ):
                    await queue.put({"model_id": model_id, "type": "chunk", "chunk": chunk})
                await queue.put({"model_id": model_id, "type": "done"})
            except Exception as e:
                logger.error(f"Error streaming from {model_id}: {e}")
                await queue.put({"model_id": model_id, "type": "error", "error": str(e)})

        # Start all model streams
        tasks = [
            asyncio.create_task(stream_model(m)) for m in prompt_request.models
        ]

        # Send initial tile info
        yield f"data: {json.dumps({'type': 'tile_created', 'tile_id': tile.id})}\n\n"

        # Process queue until all models done
        try:
            while len(model_done) < len(prompt_request.models):
                try:
                    event = await asyncio.wait_for(queue.get(), timeout=120.0)
                    model_id = event["model_id"]

                    if event["type"] == "error":
                        store.set_response_error(
                            session_id, tile.id, model_id, event["error"]
                        )
                        yield f"data: {json.dumps({'type': 'error', 'model_id': model_id, 'error': event['error']})}\n\n"
                        model_done.add(model_id)

                    elif event["type"] == "done":
                        # Save final content
                        store.update_response_content(
                            session_id,
                            tile.id,
                            model_id,
                            model_content[model_id],
                            "completed",
                        )
                        yield f"data: {json.dumps({'type': 'complete', 'model_id': model_id})}\n\n"
                        model_done.add(model_id)

                    elif event["type"] == "chunk":
                        model_content[model_id] += event["chunk"]
                        # Update store (not saving to disk yet)
                        store.update_response_content(
                            session_id,
                            tile.id,
                            model_id,
                            model_content[model_id],
                            "streaming",
                        )
                        yield f"data: {json.dumps({'type': 'chunk', 'model_id': model_id, 'chunk': event['chunk']})}\n\n"

                except asyncio.TimeoutError:
                    logger.warning("Stream timeout")
                    yield f"data: {json.dumps({'type': 'timeout'})}\n\n"
                    break

        finally:
            # Cleanup tasks
            for task in tasks:
                if not task.done():
                    task.cancel()

            # Save session after all streams complete
            store.save_session(session_id)

        yield f"data: {json.dumps({'type': 'session_saved'})}\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(
        event_generator(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
        },
    )


# --- Debate ---


@router.post("/{session_id}/debate")
@limiter.limit("10 per minute")
async def start_debate(
    session_id: str, debate_request: DebateStartRequest, request: Request
):
    """Start a debate between models (SSE streaming)"""
    store = get_canvas_store(request)
    openrouter = get_openrouter(request)

    if not openrouter.is_configured():
        raise HTTPException(
            status_code=503, detail="OpenRouter API key not configured"
        )

    session = store.get_session(session_id)
    if not session:
        raise HTTPException(status_code=404, detail="Session not found")

    # Gather source responses for debate context
    tile_data = store.get_tile_responses(session_id, debate_request.source_tile_ids)
    if not tile_data:
        raise HTTPException(status_code=400, detail="No valid source tiles found")

    # Create debate
    debate = store.add_debate(
        session_id,
        debate_request.source_tile_ids,
        debate_request.participating_models,
        debate_request.debate_mode,
    )

    if not debate:
        raise HTTPException(status_code=500, detail="Failed to create debate")

    async def debate_generator():
        """Generate SSE events for debate rounds"""
        # Build context from source tiles
        context_parts = []
        for tile_id, data in tile_data.items():
            context_parts.append(f"Original prompt: {data['prompt']}\n")
            for model_id, response in data["responses"].items():
                if model_id in debate_request.participating_models:
                    model_name = model_id.split("/")[-1]
                    context_parts.append(f"[{model_name}]: {response}\n")

        context = "\n".join(context_parts)

        # Debate system prompt
        if debate_request.debate_mode == DebateMode.AUTO:
            system_prompt = """You are participating in a multi-model debate.
You will see responses from other AI models on the same topic.
Critically analyze their responses:
- Point out strengths and weaknesses
- Identify factual errors or logical flaws
- Suggest improvements
- Defend your position if your response was included
Be constructive but thorough in your critique."""
        else:
            system_prompt = debate_request.debate_prompt or "Analyze and compare the following responses."

        yield f"data: {json.dumps({'type': 'debate_created', 'debate_id': debate.id})}\n\n"

        # Run debate rounds
        for round_num in range(debate_request.max_rounds):
            yield f"data: {json.dumps({'type': 'round_start', 'round': round_num + 1})}\n\n"

            round_responses = {}
            queue = asyncio.Queue()

            # Build messages for this round
            messages = [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": f"Here is the context:\n\n{context}\n\nProvide your analysis."},
            ]

            # Add previous round responses to context
            if debate.rounds:
                prev_round = debate.rounds[-1]
                prev_context = "\n".join(
                    f"[{m.split('/')[-1]}]: {r}" for m, r in prev_round.items()
                )
                messages.append(
                    {"role": "assistant", "content": "Previous round responses:"}
                )
                messages.append({"role": "user", "content": prev_context})

            async def stream_debate_model(model_id: str):
                """Stream debate response from a model"""
                try:
                    content = ""
                    async for chunk in openrouter.stream_completion(
                        model_id, messages, temperature=0.7, max_tokens=1024
                    ):
                        content += chunk
                        await queue.put({
                            "model_id": model_id,
                            "type": "chunk",
                            "chunk": chunk,
                        })
                    await queue.put({
                        "model_id": model_id,
                        "type": "done",
                        "content": content,
                    })
                except Exception as e:
                    await queue.put({
                        "model_id": model_id,
                        "type": "error",
                        "error": str(e),
                    })

            tasks = [
                asyncio.create_task(stream_debate_model(m))
                for m in debate_request.participating_models
            ]

            model_done = set()
            try:
                while len(model_done) < len(debate_request.participating_models):
                    event = await asyncio.wait_for(queue.get(), timeout=120.0)
                    model_id = event["model_id"]

                    if event["type"] == "chunk":
                        yield f"data: {json.dumps({'type': 'debate_chunk', 'round': round_num + 1, 'model_id': model_id, 'chunk': event['chunk']})}\n\n"

                    elif event["type"] == "done":
                        round_responses[model_id] = event["content"]
                        yield f"data: {json.dumps({'type': 'model_complete', 'round': round_num + 1, 'model_id': model_id})}\n\n"
                        model_done.add(model_id)

                    elif event["type"] == "error":
                        yield f"data: {json.dumps({'type': 'debate_error', 'round': round_num + 1, 'model_id': model_id, 'error': event['error']})}\n\n"
                        model_done.add(model_id)

            except asyncio.TimeoutError:
                yield f"data: {json.dumps({'type': 'timeout', 'round': round_num + 1})}\n\n"
            finally:
                for task in tasks:
                    if not task.done():
                        task.cancel()

            # Save round
            if round_responses:
                store.add_debate_round(session_id, debate.id, round_responses)
                context += "\n\nPrevious debate round:\n" + "\n".join(
                    f"[{m.split('/')[-1]}]: {r}" for m, r in round_responses.items()
                )

            yield f"data: {json.dumps({'type': 'round_complete', 'round': round_num + 1})}\n\n"

            # For auto mode, check if we should continue
            if debate_request.debate_mode == DebateMode.MEDIATED:
                break  # Only one round in mediated mode, user continues manually

        store.update_debate_status(session_id, debate.id, "completed")
        store.save_session(session_id)

        yield f"data: {json.dumps({'type': 'debate_complete', 'debate_id': debate.id})}\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(
        debate_generator(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
        },
    )


@router.post("/{session_id}/debate/{debate_id}/continue")
@limiter.limit("10 per minute")
async def continue_debate(
    session_id: str,
    debate_id: str,
    continue_request: DebateContinueRequest,
    request: Request,
):
    """Continue a debate with a custom prompt (user-mediated mode)"""
    store = get_canvas_store(request)
    openrouter = get_openrouter(request)

    if not openrouter.is_configured():
        raise HTTPException(
            status_code=503, detail="OpenRouter API key not configured"
        )

    session = store.get_session(session_id)
    if not session:
        raise HTTPException(status_code=404, detail="Session not found")

    # Find the debate
    debate = None
    for d in session.debates:
        if d.id == debate_id:
            debate = d
            break

    if not debate:
        raise HTTPException(status_code=404, detail="Debate not found")

    if debate.status == "completed":
        # Reactivate for continuation
        store.update_debate_status(session_id, debate_id, "active")

    async def continue_generator():
        """Generate SSE events for continued debate"""
        # Build context from previous rounds
        context_parts = []
        for i, round_data in enumerate(debate.rounds):
            context_parts.append(f"Round {i + 1}:")
            for model_id, response in round_data.items():
                model_name = model_id.split("/")[-1]
                context_parts.append(f"[{model_name}]: {response}")

        context = "\n".join(context_parts)

        messages = [
            {"role": "system", "content": "You are participating in a continued debate discussion."},
            {"role": "user", "content": f"Previous discussion:\n\n{context}\n\nNew instruction: {continue_request.prompt}"},
        ]

        round_num = len(debate.rounds) + 1
        yield f"data: {json.dumps({'type': 'round_start', 'round': round_num})}\n\n"

        round_responses = {}
        queue = asyncio.Queue()

        async def stream_model(model_id: str):
            try:
                content = ""
                async for chunk in openrouter.stream_completion(
                    model_id, messages, temperature=0.7, max_tokens=1024
                ):
                    content += chunk
                    await queue.put({
                        "model_id": model_id,
                        "type": "chunk",
                        "chunk": chunk,
                    })
                await queue.put({
                    "model_id": model_id,
                    "type": "done",
                    "content": content,
                })
            except Exception as e:
                await queue.put({
                    "model_id": model_id,
                    "type": "error",
                    "error": str(e),
                })

        tasks = [
            asyncio.create_task(stream_model(m))
            for m in debate.participating_models
        ]

        model_done = set()
        try:
            while len(model_done) < len(debate.participating_models):
                event = await asyncio.wait_for(queue.get(), timeout=120.0)
                model_id = event["model_id"]

                if event["type"] == "chunk":
                    yield f"data: {json.dumps({'type': 'debate_chunk', 'round': round_num, 'model_id': model_id, 'chunk': event['chunk']})}\n\n"

                elif event["type"] == "done":
                    round_responses[model_id] = event["content"]
                    yield f"data: {json.dumps({'type': 'model_complete', 'round': round_num, 'model_id': model_id})}\n\n"
                    model_done.add(model_id)

                elif event["type"] == "error":
                    yield f"data: {json.dumps({'type': 'debate_error', 'round': round_num, 'model_id': model_id, 'error': event['error']})}\n\n"
                    model_done.add(model_id)

        except asyncio.TimeoutError:
            yield f"data: {json.dumps({'type': 'timeout', 'round': round_num})}\n\n"
        finally:
            for task in tasks:
                if not task.done():
                    task.cancel()

        if round_responses:
            store.add_debate_round(session_id, debate_id, round_responses)

        store.save_session(session_id)

        yield f"data: {json.dumps({'type': 'round_complete', 'round': round_num})}\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(
        continue_generator(),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
        },
    )


@router.put("/{session_id}/debate/{debate_id}/status")
async def update_debate_status(
    session_id: str, debate_id: str, status: str, request: Request
):
    """Update debate status (pause/resume/complete)"""
    store = get_canvas_store(request)

    if status not in ["active", "paused", "completed"]:
        raise HTTPException(status_code=400, detail="Invalid status")

    if not store.update_debate_status(session_id, debate_id, status):
        raise HTTPException(status_code=404, detail="Debate not found")

    return {"status": "updated"}
