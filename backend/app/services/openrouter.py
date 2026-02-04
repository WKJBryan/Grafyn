"""OpenRouter API client with streaming support"""
import httpx
import json
import logging
from typing import AsyncGenerator, List, Dict, Optional
from datetime import datetime, timezone, timedelta

from app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()


class OpenRouterService:
    """Service for interacting with OpenRouter API"""

    BASE_URL = "https://openrouter.ai/api/v1"

    def __init__(self):
        self.api_key = settings.openrouter_api_key
        self._http_client: Optional[httpx.AsyncClient] = None
        self._models_cache: Optional[List[Dict]] = None
        self._cache_expiry: Optional[datetime] = None

    async def _get_client(self) -> httpx.AsyncClient:
        """Get or create HTTP client"""
        if self._http_client is None or self._http_client.is_closed:
            self._http_client = httpx.AsyncClient(
                timeout=httpx.Timeout(120.0, connect=10.0),
                headers={
                    "Authorization": f"Bearer {self.api_key}",
                    "HTTP-Referer": settings.app_url,
                    "X-Title": "Grafyn Knowledge Platform",
                    "Content-Type": "application/json",
                },
            )
        return self._http_client

    async def close(self):
        """Close HTTP client"""
        if self._http_client and not self._http_client.is_closed:
            await self._http_client.aclose()
            self._http_client = None

    def is_configured(self) -> bool:
        """Check if OpenRouter API key is configured"""
        return bool(self.api_key)

    async def list_models(self) -> List[Dict]:
        """Get available models from OpenRouter with caching"""
        now = datetime.now(timezone.utc)

        # Return cached if valid
        if (
            self._models_cache is not None
            and self._cache_expiry is not None
            and now < self._cache_expiry
        ):
            return self._models_cache

        if not self.is_configured():
            logger.warning("OpenRouter API key not configured")
            return []

        try:
            client = await self._get_client()
            response = await client.get(f"{self.BASE_URL}/models")
            response.raise_for_status()

            data = response.json()
            models = data.get("data", [])

            # Process and filter models
            processed_models = []
            for model in models:
                processed_models.append(
                    {
                        "id": model.get("id", ""),
                        "name": model.get("name", model.get("id", "").split("/")[-1]),
                        "provider": model.get("id", "").split("/")[0]
                        if "/" in model.get("id", "")
                        else "unknown",
                        "context_length": model.get("context_length", 4096),
                        "pricing": model.get("pricing", {}),
                        "supports_streaming": True,
                    }
                )

            self._models_cache = processed_models
            self._cache_expiry = now + timedelta(minutes=5)

            logger.info(f"Loaded {len(processed_models)} models from OpenRouter")
            return processed_models

        except httpx.HTTPStatusError as e:
            logger.error(f"OpenRouter API error: {e.response.status_code}")
            return self._models_cache or []
        except Exception as e:
            logger.error(f"Failed to fetch models from OpenRouter: {e}")
            return self._models_cache or []

    async def stream_completion(
        self,
        model_id: str,
        messages: List[Dict[str, str]],
        temperature: float = 0.7,
        max_tokens: int = 2048,
    ) -> AsyncGenerator[str, None]:
        """Stream a chat completion from OpenRouter"""
        if not self.is_configured():
            raise ValueError("OpenRouter API key not configured")

        client = await self._get_client()

        payload = {
            "model": model_id,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": True,
        }

        try:
            async with client.stream(
                "POST", f"{self.BASE_URL}/chat/completions", json=payload
            ) as response:
                response.raise_for_status()

                async for line in response.aiter_lines():
                    if not line:
                        continue

                    if line.startswith("data: "):
                        data = line[6:]

                        if data == "[DONE]":
                            break

                        try:
                            chunk = json.loads(data)

                            # Skip non-content chunks
                            if "choices" not in chunk or not chunk["choices"]:
                                continue

                            delta = chunk["choices"][0].get("delta", {})
                            content = delta.get("content", "")

                            if content:
                                yield content

                        except json.JSONDecodeError:
                            continue

        except httpx.HTTPStatusError as e:
            error_msg = f"API error: {e.response.status_code}"
            try:
                error_data = e.response.json()
                if "error" in error_data:
                    error_msg = error_data["error"].get("message", error_msg)
            except Exception:
                pass
            raise RuntimeError(error_msg)
        except httpx.TimeoutException:
            raise RuntimeError("Request timed out")
        except Exception as e:
            raise RuntimeError(f"Stream error: {str(e)}")

    async def complete(
        self,
        model_id: str,
        messages: List[Dict[str, str]],
        temperature: float = 0.7,
        max_tokens: int = 2048,
    ) -> str:
        """Non-streaming completion (collects full response)"""
        content = ""
        async for chunk in self.stream_completion(
            model_id, messages, temperature, max_tokens
        ):
            content += chunk
        return content

    async def validate_api_key(self) -> bool:
        """Validate the API key by making a test request"""
        if not self.is_configured():
            return False

        try:
            await self.list_models()
            return True
        except Exception:
            return False
