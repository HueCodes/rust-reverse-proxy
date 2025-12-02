# Deployment Guide

## Local Testing with Docker

### Prerequisites
- Docker or OrbStack installed and running

### Build and Run
```bash
# Build all services
docker-compose build

# Start the stack
docker-compose up -d

# View logs
docker-compose logs -f

# Test the proxy
curl http://localhost:3000/test

# Check health endpoint
curl http://localhost:3000/health

# Stop the stack
docker-compose down
```

### Test Round-Robin Load Balancing
```bash
# Make several requests and see them distributed across backends
for i in {1..10}; do
  curl -s http://localhost:3000/test | jq '.server'
done
```

## Deploying to a VM

### 1. Provision a VM
Choose a cloud provider:
- **DigitalOcean** ($6/month droplet)
- **Linode** ($5/month nanode)
- **AWS EC2** (t2.micro free tier)
- **Hetzner** (€4/month)

### 2. Connect to VM
```bash
ssh root@your-vm-ip
```

### 3. Install Docker
```bash
# Update system
apt update && apt upgrade -y

# Install Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh

# Install Docker Compose
apt install docker-compose -y
```

### 4. Copy Project Files
```bash
# From your local machine
scp -r /path/to/rust-reverse-proxy root@your-vm-ip:/root/

# Or clone from git
git clone your-repo-url
cd rust-reverse-proxy
```

### 5. Start the Services
```bash
docker-compose up -d
```

### 6. Configure Firewall
```bash
# Allow HTTP/HTTPS
ufw allow 80/tcp
ufw allow 443/tcp
ufw allow 22/tcp  # SSH
ufw enable
```

## Adding TLS with Caddy

Create a `docker-compose.caddy.yml`:

```yaml
version: '3.8'

services:
  caddy:
    image: caddy:2-alpine
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
      - caddy_config:/config
    networks:
      - proxy-network
    restart: unless-stopped

volumes:
  caddy_data:
  caddy_config:

networks:
  proxy-network:
    external: true
```

Create a `Caddyfile`:
```
yourdomain.com {
    reverse_proxy proxy:3000
}
```

Start with:
```bash
docker-compose -f docker-compose.yml -f docker-compose.caddy.yml up -d
```

## Monitoring

### View Logs
```bash
# All services
docker-compose logs -f

# Specific service
docker-compose logs -f proxy

# Last 100 lines
docker-compose logs --tail=100 proxy
```

### Check Status
```bash
docker-compose ps
```

### Resource Usage
```bash
docker stats
```

## Maintenance

### Update and Restart
```bash
# Pull latest code
git pull

# Rebuild and restart
docker-compose up -d --build

# Remove old images
docker image prune -a
```

### Backup
```bash
# Backup config
cp config.docker.yaml config.docker.yaml.backup

# Backup entire directory
tar -czf reverse-proxy-backup.tar.gz /root/rust-reverse-proxy
```

## Troubleshooting

### Containers Won't Start
```bash
# Check logs
docker-compose logs

# Check if ports are in use
netstat -tulpn | grep 3000

# Restart everything
docker-compose down && docker-compose up -d
```

### Health Checks Failing
```bash
# Test backend directly
docker exec backend1 curl localhost:8001/health
docker exec backend2 curl localhost:8002/health

# Test proxy
docker exec rust-reverse-proxy curl localhost:3000/health
```

### High Memory Usage
```bash
# Check resource usage
docker stats

# Restart services
docker-compose restart
```
