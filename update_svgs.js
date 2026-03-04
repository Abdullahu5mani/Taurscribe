const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, 'generated_logos');

// 1. First, create the no-background static logo.
const staticPath = path.join(dir, 'static_logo_layered_core.svg');
let staticContent = fs.readFileSync(staticPath, 'utf8');

// Remove the strict dark rect background
staticContent = staticContent.replace(/<rect width="500" height="500" rx="100" fill="#181514"[^>]*>[\s\S]*?(?:<\/rect>)?/, '');

// Save it as a new file
fs.writeFileSync(path.join(dir, 'static_logo_layered_nobg.svg'), staticContent);
console.log('Created static_logo_layered_nobg.svg');

// 2. Process all animated svgs
let count = 0;
for (const file of fs.readdirSync(dir)) {
    if (!file.startsWith('animated_') || !file.endsWith('.svg')) continue;

    const p = path.join(dir, file);
    let c = fs.readFileSync(p, 'utf8');

    // Guard against double processing scale
    if (!c.includes('scale(1.55)')) {
        // Replace main g transform and stroke width
        c = c.replace(/<g\s+transform="translate\(250,\s*160\)"[^>]*stroke-width="6"/g, (match) => {
            return match
                .replace('translate(250, 160)', 'translate(250, 165) scale(1.55)')
                .replace('stroke-width="6"', 'stroke-width="3.8"');
        });

        // Special case for blueprint
        c = c.replace(/<g\s+transform="translate\(250,\s*160\)"\s+stroke-width="6"/g, (match) => {
            return match
                .replace('translate(250, 160)', 'translate(250, 165) scale(1.55)')
                .replace('stroke-width="6"', 'stroke-width="3.8"');
        });

        // Replace pencil group stroke width from 4 to 2.5
        c = c.replace(/stroke-width="4"/g, 'stroke-width="2.5"');
        c = c.replace(/stroke-width="3"/g, 'stroke-width="1.9"'); // blueprint line

        if (file === 'animated_logo_stomp.svg') {
            // Adjust stomp animation safely without breaking SVG
            c = c.replace(
                /<animateTransform attributeName="transform" type="translate"\s*values="250,160; 240,165; 260,155; 245,162; 255,158; 250,160"/,
                '<animateTransform attributeName="transform" type="translate" values="0,0; -10,5; 10,-5; -5,2; 5,-2; 0,0" additive="sum"'
            );
            // And inject a wrapping group to receive the stomp translation sum safely before the scaling group
            c = c.replace(/<g transform="translate\(250, 165\) scale\(1.55\)"/, '<g>\n  <g transform="translate(250, 165) scale(1.55)"');
            c = c.replace(/<\/g>\s*<\/svg>/, '</g>\n</g>\n</svg>');
        }
    }

    // Remove the dark rect background block (and its inner <animate> tag) from ALL of them
    c = c.replace(/<rect width="500" height="500" rx="100" fill="#181514">\s*<animate[^>]*>\s*<\/rect>\s*/, '');

    fs.writeFileSync(p, c);
    count++;
}

console.log(`Updated scale and removed backgrounds from ${count} animated SVG files.`);
