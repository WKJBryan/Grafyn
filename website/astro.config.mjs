import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://wkjbryan.github.io',
  base: '/Grafyn/',
  vite: {
    plugins: [tailwindcss()],
  },
});
