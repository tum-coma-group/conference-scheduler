#!/bin/bash

echo "🚀 Starting Conference Scheduler..."

# Create data directories if they don't exist
mkdir -p data/uploads data/generated

# Build and start the container
docker-compose up --build

echo "✅ Conference Scheduler is running at http://localhost:3000"
echo "📁 Uploaded files will be stored in ./data/uploads"
echo "📄 Generated schedules will be stored in ./data/generated"