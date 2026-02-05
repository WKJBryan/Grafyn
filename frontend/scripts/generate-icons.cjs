// Generate a proper RGBA PNG source icon for Tauri
// After running this script, use `npx tauri icon` to generate all platform formats
const sharp = require('sharp');
const fs = require('fs');
const path = require('path');

const iconsDir = path.join(__dirname, '../src-tauri/icons');

// Create icons directory if it doesn't exist
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Create a placeholder icon (gradient square with "G" letter)
async function createPlaceholder(size) {
  const svg = `
    <svg width="${size}" height="${size}" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style="stop-color:#4F46E5;stop-opacity:1" />
          <stop offset="100%" style="stop-color:#7C3AED;stop-opacity:1" />
        </linearGradient>
      </defs>
      <rect width="${size}" height="${size}" rx="${size * 0.15}" fill="url(#grad)"/>
      <text x="50%" y="55%" font-family="Arial, sans-serif" font-size="${size * 0.4}"
            fill="white" text-anchor="middle" dominant-baseline="middle" font-weight="bold">G</text>
    </svg>
  `;

  return sharp(Buffer.from(svg))
    .ensureAlpha()
    .png()
    .toBuffer();
}

async function main() {
  try {
    const appIconPath = path.join(iconsDir, 'app-icon.png');
    let sourceBuffer;

    if (fs.existsSync(appIconPath)) {
      console.log('Found existing app-icon.png, checking format...');
      // Read file into buffer first to avoid file handle conflicts on Windows
      const inputBuffer = fs.readFileSync(appIconPath);
      const meta = await sharp(inputBuffer).metadata();
      console.log(`  Format: ${meta.format}, Channels: ${meta.channels}, Alpha: ${meta.hasAlpha}, Size: ${meta.width}x${meta.height}`);

      // Convert to proper 1024x1024 RGBA PNG regardless of input format
      sourceBuffer = await sharp(inputBuffer)
        .resize(1024, 1024)
        .ensureAlpha()
        .png()
        .toBuffer();
    } else {
      console.log('No app-icon.png found, generating placeholder...');
      sourceBuffer = await createPlaceholder(1024);
    }

    // Write the canonical RGBA PNG source icon
    fs.writeFileSync(appIconPath, sourceBuffer);

    // Verify the output
    const outMeta = await sharp(sourceBuffer).metadata();
    console.log(`\nWritten app-icon.png: ${outMeta.width}x${outMeta.height}, format=${outMeta.format}, channels=${outMeta.channels}, hasAlpha=${outMeta.hasAlpha}`);

    if (outMeta.channels !== 4 || !outMeta.hasAlpha) {
      console.error('ERROR: app-icon.png is not RGBA! Tauri builds will fail.');
      process.exit(1);
    }

    console.log('\napp-icon.png is ready. Now run:');
    console.log('  npx tauri icon src-tauri/icons/app-icon.png');
    console.log('\nOr use the combined command:');
    console.log('  npm run generate-icons');
  } catch (err) {
    console.error('Error:', err);
    process.exit(1);
  }
}

main();
