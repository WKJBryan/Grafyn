# Seedream Frontend

A Vue 3 Single Page Application for the Seedream knowledge management system.

## Tech Stack

- **Framework**: Vue 3.4+ (Composition API)
- **Build Tool**: Vite 5.0+
- **State Management**: Pinia
- **Routing**: Vue Router 4.2+
- **HTTP Client**: Axios 1.6+
- **Markdown**: marked 11.0+
- **Language**: JavaScript (with JSDoc for type hints)

## Features

- Note listing and navigation
- Markdown editing with live preview
- Semantic search with typeahead
- Backlink visualization
- Clean dark theme design system
- OAuth authentication (GitHub, Google)
- Responsive layout

## Project Structure

```
frontend/
├── index.html              # HTML entry point
├── package.json            # Dependencies and scripts
├── vite.config.js          # Vite configuration
├── jsconfig.json           # JavaScript configuration
├── .eslintrc.cjs          # ESLint configuration
├── .prettierrc            # Prettier configuration
├── .gitignore             # Git ignore rules
└── src/
    ├── main.js             # Vue app bootstrap
    ├── App.vue             # Root component
    ├── style.css           # Design system and global styles
    ├── api/
    │   └── client.js       # Backend API client
    ├── components/
    │   ├── SearchBar.vue   # Semantic search component
    │   ├── NoteList.vue    # Sidebar note listing
    │   ├── NoteEditor.vue  # Markdown editor
    │   ├── BacklinksPanel.vue  # Backlinks panel
    │   └── GraphView.vue   # Graph visualization (placeholder)
    ├── views/
    │   ├── HomeView.vue    # Main application view
    │   ├── LoginView.vue   # Login page
    │   ├── OAuthCallbackView.vue  # OAuth callback handler
    │   └── NotFoundView.vue # 404 page
    ├── stores/
    │   ├── notes.js        # Notes state management
    │   └── auth.js        # Authentication state management
    └── router/
        └── index.js        # Vue Router configuration
```

## Getting Started

### Prerequisites

- Node.js 18+ or 20+
- npm, yarn, or pnpm
- Backend API running at `http://localhost:8080`

### Installation

1. Install dependencies:

```bash
npm install
```

2. Start the development server:

```bash
npm run dev
```

The application will be available at `http://localhost:5173`

### Available Scripts

- `npm run dev` - Start development server
- `npm run build` - Build for production
- `npm run preview` - Preview production build locally
- `npm run lint` - Run ESLint
- `npm run format` - Format code with Prettier

## API Integration

The frontend connects to the backend API at `http://localhost:8080` via Vite proxy configuration.

### API Endpoints

The API client (`src/api/client.js`) provides methods for:

- **Notes**: CRUD operations, reindexing
- **Search**: Semantic search, similar notes
- **Graph**: Backlinks, outgoing links, neighbors
- **OAuth**: Authorization, callback handling, user info

### Authentication

OAuth authentication is supported via:
- GitHub
- Google

Authentication tokens are stored in localStorage and automatically included in API requests.

## Development

### Code Style

The project uses ESLint and Prettier for code formatting and linting. Run `npm run lint` to check for issues and `npm run format` to fix formatting.

### Design System

The design system is defined in `src/style.css` with CSS custom properties for:
- Colors (primary, secondary, accents)
- Spacing
- Typography
- Border radius
- Transitions

### Component Architecture

Components follow Vue 3 Composition API patterns with:
- `<script setup>` syntax
- Reactive state with `ref` and `computed`
- Props and emits for communication
- Scoped styles

### State Management

Pinia stores are used for:
- `useNotesStore`: Notes data and operations
- `useAuthStore`: Authentication state and methods

## Building for Production

```bash
npm run build
```

The production build will be in the `dist/` directory.

## Environment Variables

No environment variables are required for development. The backend API URL is configured in `vite.config.js`.

For production deployment, you may need to configure:
- `VITE_API_BASE_URL` - Backend API URL
- `VITE_OAUTH_REDIRECT_URI` - OAuth callback URL

## Troubleshooting

### Backend Connection Issues

If the frontend cannot connect to the backend:
1. Ensure the backend is running at `http://localhost:8080`
2. Check the Vite proxy configuration in `vite.config.js`
3. Check browser console for CORS errors

### OAuth Issues

If OAuth authentication fails:
1. Verify OAuth credentials are configured in the backend
2. Check the redirect URI matches the frontend URL
3. Review browser console for error messages

## License

This project is part of the Seedream knowledge management system.
