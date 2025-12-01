#!/bin/bash

# Test script for health endpoint

echo "Testing health endpoint at http://localhost:3000/health"
echo ""

response=$(curl -s http://localhost:3000/health)
echo "$response" | jq '.' 2>/dev/null || echo "$response"

echo ""
echo "Health endpoint should return JSON with:"
echo "  - status: healthy/unhealthy"
echo "  - uptime_seconds: server uptime"
echo "  - backends: array of backend health status"
