// Generate placeholder icons for Tauri
const sharp = require('sharp');
const fs = require('fs');
const path = require('path');

const iconsDir = path.join(__dirname, '../src-tauri/icons');

// Create icons directory if it doesn't exist
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Create a simple gradient icon as placeholder
async function createIcon(size, filename) {
  // Create a colored square with gradient effect
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
            fill="white" text-anchor="middle" dominant-baseline="middle" font-weight="bold">S</text>
    </svg>
  `;

  await sharp(Buffer.from(svg))
    .png()
    .toFile(path.join(iconsDir, filename));

  console.log(`Created ${filename}`);
}

async function createIco() {
  // Create multiple sizes for ICO
  const sizes = [16, 32, 48, 64, 128, 256];
  const buffers = [];

  for (const size of sizes) {
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
              fill="white" text-anchor="middle" dominant-baseline="middle" font-weight="bold">S</text>
      </svg>
    `;

    const buf = await sharp(Buffer.from(svg)).png().toBuffer();
    buffers.push({ size, buf });
  }

  // Create ICO file manually (simplified - just use the 256x256 PNG for now)
  // For a proper ICO, we'd need a more complex implementation
  const largest = buffers.find(b => b.size === 256);
  await sharp(largest.buf).toFile(path.join(iconsDir, 'icon.ico'));
  console.log('Created icon.ico (as PNG - may need conversion)');
}

async function main() {
  try {
    // Generate PNG icons
    await createIcon(32, '32x32.png');
    await createIcon(128, '128x128.png');
    await createIcon(256, '128x128@2x.png');
    await createIcon(512, 'icon.png');
    await createIcon(1024, 'app-icon.png');

    // For ICO, just copy the 256 PNG (Tauri's icon command can convert it)
    const icon256 = await sharp(path.join(iconsDir, '128x128@2x.png')).toBuffer();
    fs.writeFileSync(path.join(iconsDir, 'icon.ico'), icon256);
    console.log('Created icon.ico (PNG format - use npm run tauri icon to convert)');

    // For ICNS (macOS), just use PNG
    fs.copyFileSync(path.join(iconsDir, 'icon.png'), path.join(iconsDir, 'icon.icns'));
    console.log('Created icon.icns (PNG placeholder)');

    console.log('\nDone! Icons created in src-tauri/icons/');
    console.log('For proper ICO/ICNS files, run: npm run tauri icon src-tauri/icons/app-icon.png');
  } catch (err) {
    console.error('Error:', err);
  }
}

main();
