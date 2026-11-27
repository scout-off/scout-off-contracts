FROM node:20-slim AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci --ignore-scripts
COPY . .
RUN npm run build 2>/dev/null || npx tsc --project tsconfig.json || true

FROM node:20-slim
WORKDIR /app
ENV NODE_ENV=production
COPY package*.json ./
RUN npm ci --omit=dev --ignore-scripts
COPY --from=builder /app/dist ./dist
COPY prisma ./prisma
COPY migrations ./migrations
EXPOSE 3000
CMD ["node", "dist/src/server.js"]
