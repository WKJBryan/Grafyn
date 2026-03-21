/**
 * Grafyn Worker
 *
 * Serves Tauri auto-update assets from Cloudflare R2 and proxies
 * feedback submissions to GitHub Issues.
 *
 * Routes:
 *   GET  /latest.json              → Update manifest (5-min cache)
 *   GET  /download/:version/:file  → Installer binary (1-hour cache)
 *   POST /feedback                 → Proxy to GitHub Issues API
 *   GET  /                         → Health check
 *
 * Secrets (set via `wrangler secret put`):
 *   GITHUB_FEEDBACK_TOKEN  — GitHub PAT with issues:write scope
 *   FEEDBACK_KEY           — Shared anti-abuse key checked via X-Feedback-Key header
 *
 * Env vars (set in wrangler.toml):
 *   FEEDBACK_REPO          — GitHub repo in "owner/repo" format
 */

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, {
        headers: corsHeaders(),
      });
    }

    try {
      // Route: POST /feedback
      if (request.method === 'POST' && path === '/feedback') {
        return await handleFeedback(request, env);
      }

      // All other routes are GET-only
      if (request.method !== 'GET') {
        return jsonResponse({ error: 'Method not allowed' }, 405);
      }

      // Route: GET /latest.json
      if (path === '/latest.json') {
        return await serveObject(env, 'latest.json', {
          'Content-Type': 'application/json',
          'Cache-Control': 'public, max-age=300', // 5-minute cache
        });
      }

      // Route: GET /download/:version/:filename
      const downloadMatch = path.match(/^\/download\/([^/]+)\/(.+)$/);
      if (downloadMatch) {
        const [, version, filename] = downloadMatch;
        const key = `${version}/${filename}`;
        return await serveObject(env, key, {
          'Content-Type': 'application/octet-stream',
          'Content-Disposition': `attachment; filename="${filename}"`,
          'Cache-Control': 'public, max-age=3600', // 1-hour cache
        });
      }

      // Route: GET / — health check
      if (path === '/') {
        return jsonResponse({
          service: 'grafyn-updater',
          status: 'ok',
        });
      }

      return jsonResponse({ error: 'Not found' }, 404);
    } catch (err) {
      return jsonResponse({ error: 'Internal server error' }, 500);
    }
  },
};

/**
 * Handle POST /feedback — validate, authenticate, proxy to GitHub Issues API.
 */
async function handleFeedback(request, env) {
  // Check anti-abuse key
  const feedbackKey = request.headers.get('X-Feedback-Key');
  if (!feedbackKey || feedbackKey !== env.FEEDBACK_KEY) {
    return jsonResponse({ error: 'Unauthorized' }, 401);
  }

  // Check worker has required config
  if (!env.GITHUB_FEEDBACK_TOKEN || !env.FEEDBACK_REPO) {
    return jsonResponse({ error: 'Feedback service not configured' }, 503);
  }

  // Parse and validate request body
  let body;
  try {
    body = await request.json();
  } catch {
    return jsonResponse({ error: 'Invalid JSON body' }, 400);
  }

  if (!body.title || typeof body.title !== 'string' || body.title.trim().length === 0) {
    return jsonResponse({ error: 'title is required' }, 400);
  }
  if (!body.body || typeof body.body !== 'string' || body.body.trim().length === 0) {
    return jsonResponse({ error: 'body is required' }, 400);
  }
  if (body.labels && !Array.isArray(body.labels)) {
    return jsonResponse({ error: 'labels must be an array' }, 400);
  }

  // Forward to GitHub Issues API
  const githubUrl = `https://api.github.com/repos/${env.FEEDBACK_REPO}/issues`;

  const githubResponse = await fetch(githubUrl, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${env.GITHUB_FEEDBACK_TOKEN}`,
      'User-Agent': 'Grafyn-Feedback-Worker',
      'Accept': 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      title: body.title,
      body: body.body,
      labels: body.labels || [],
    }),
  });

  const githubData = await githubResponse.json();

  if (!githubResponse.ok) {
    return jsonResponse(
      { error: 'GitHub API error', status: githubResponse.status, detail: githubData.message },
      502,
    );
  }

  return jsonResponse({
    number: githubData.number,
    html_url: githubData.html_url,
  });
}

async function serveObject(env, key, headers) {
  const object = await env.RELEASES.get(key);

  if (!object) {
    return jsonResponse({ error: 'Not found', key }, 404);
  }

  return new Response(object.body, {
    headers: {
      ...corsHeaders(),
      ...headers,
      'ETag': object.httpEtag,
    },
  });
}

function jsonResponse(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      'Content-Type': 'application/json',
      ...corsHeaders(),
    },
  });
}

function corsHeaders() {
  return {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type, X-Feedback-Key',
  };
}
