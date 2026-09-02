# Docker Setup for Conference Scheduler

A simple Docker setup with persistent filesystem storage for file uploads.

## Quick Start

1. **Start the application:**
   ```bash
   ./start.sh
   ```

   Or manually:
   ```bash
   mkdir -p data/uploads data/generated
   docker-compose up --build
   ```

2. **Access the web interface:**
   Open http://localhost:3000

## File Storage

- **Uploaded files**: Stored in `./data/uploads/`
- **Generated schedules**: Stored in `./data/generated/`

These directories are created automatically and persist across container restarts.

## Management Commands

- **Start**: `docker-compose up -d`
- **Stop**: `docker-compose down`
- **View logs**: `docker-compose logs -f`
- **Rebuild**: `docker-compose up --build`

## Port

The application runs on port **3000**.

## Important Notes

- All uploaded files are preserved on your host filesystem
- Generated schedules are also preserved
- Container runs as a non-root user for security
- Data persists even if you remove and recreate the container