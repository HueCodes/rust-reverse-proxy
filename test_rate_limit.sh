#!/bin/bash

# Test script for rate limiting
# This will make rapid requests to test the rate limiter

echo "Testing rate limiter..."
echo "Making 10 requests rapidly to http://localhost:3000"
echo ""

for i in {1..10}; do
    response=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/)
    echo "Request $i: HTTP $response"
    sleep 0.1
done

echo ""
echo "If rate limiting is working (100 req/60s), all should be 200 or 503"
echo "To trigger rate limiting, increase the loop count or decrease the rate limit in config.yaml"
