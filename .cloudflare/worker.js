/**
 * Grafyn Auto-Update Worker
 *
 * Serves Tauri auto-update assets from Cloudflare R2.
 * Routes:
 *   GET /latest.json              → Update manifest (5-min cache)
 *   GET /download/:version/:file  → Installer binary (1-hour cache)
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

    // Only allow GET requests
    if (request.method !== 'GET') {
      return jsonResponse({ error: 'Method not allowed' }, 405);
    }

    try {
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
    'Access-Control-Allow-Methods': 'GET, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type',
  };
}
