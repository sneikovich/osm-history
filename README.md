# history

Журнал запитів до [osm-fetcher](../osm-fetcher/README.md) у PostgreSQL. Сервіс єдиний пише в БД. Події приймає по HTTP від `overpass-server` (fire-and-forget), stateless, скейлиться репліками.

```
fetcher ──POST /events──► history ──► postgres
```

## Збірка й тести

```sh
cargo build --release
cargo test                                   # без БД: валідація, розбір User-Agent

podman run -d --rm --name pg -e POSTGRES_USER=history -e POSTGRES_PASSWORD=test \
  -e POSTGRES_DB=history -p 55432:5432 docker.io/library/postgres:17-alpine
DATABASE_URL=postgres://history:test@127.0.0.1:55432/history cargo test -- --ignored
```

## Запуск

```sh
DATABASE_URL=postgres://history:test@127.0.0.1:55432/history cargo run
```

| змінна | прапорець | за замовчуванням |
|---|---|---|
| `DATABASE_URL` | `--database-url` | обов'язкова |
| `HISTORY_LISTEN` | `--listen` | `0.0.0.0:8081` |
| `HISTORY_CONNECT_WAIT` | `--connect-wait` | `30` с: скільки чекати БД на старті |

На старті сервіс застосовує міграції з `migrations/`. Вони вбудовані в бінарник і виконуються під advisory lock, тому кілька реплік одночасно стартують безпечно. Сервер коректно завершується по SIGTERM/SIGINT.

Фетчер вмикає відправку подій через `HISTORY_URL=http://history:8081`.

## API

### `POST /events` → 204

```sh
curl -i localhost:8081/events -H 'content-type: application/json' -d '{
  "tags": ["amenity=cafe"], "area_type": "around", "coords": [50.4501, 30.5234, 300],
  "kind": "node", "status": 200, "element_count": 18, "duration_ms": 4447,
  "user_agent": "Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0"
}'
```

| поле | тип |
|---|---|
| `tags` | `string[]`, ≤ 32, кожен ≤ 256 байт |
| `area_type` | `"none"` / `"around"` / `"bbox"` |
| `coords` | `null` для `none`; 3 числа для `around` (`lat,lon,radius_m`); 4 для `bbox` (`s,w,n,e`) |
| `kind` | `"node"` / `"way"` / `"relation"` / `null` |
| `status` | HTTP-статус, яким відповів фетчер |
| `element_count` | `null`, якщо запит не вдався |
| `duration_ms` | ціле ≥ 0 |
| `user_agent` | рядок або `null`; розбирається на OS і браузер, сирий рядок не зберігається |

Невідомі поля → 422. Невідповідність `coords`/`area_type` → 422. БД недоступна → 503.

### `GET /healthz`

`200 ok`, якщо `SELECT 1` проходить, інакше 503.

## Схема

`migrations/0001_queries.sql`, таблиця `queries`:

| колонка | тип |
|---|---|
| `id` | `bigserial` |
| `created_at` | `timestamptz`, `now()` |
| `tags` | `text[]` |
| `area_type` | `text` |
| `coords` | `double precision[]` |
| `kind` | `text` |
| `status` | `smallint` |
| `element_count` | `integer` |
| `duration_ms` | `integer` |
| `client_os` | `text`: Windows, Android, iOS, ChromeOS, macOS, OpenBSD, FreeBSD, Linux |
| `client_browser` | `text`: Edge, Opera, Firefox, Chrome, Safari, curl |

IP і сирий User-Agent не зберігаються. Нерозпізнане значення → `NULL`. Правила розбору: `src/ua.rs`.

Нова міграція: файл `migrations/0002_<опис>.sql`. Вже застосовані файли не змінювати: sqlx звіряє контрольні суми й не стартує, якщо файл змінився.

## Запити

```sh
podman exec -it app_postgres_1 psql -U history
```

```sql
-- останні 20
SELECT created_at, tags, area_type, coords, kind, status, element_count, duration_ms, client_os, client_browser
FROM queries ORDER BY created_at DESC LIMIT 20;

-- популярні теги
SELECT tag, count(*) FROM queries, unnest(tags) AS tag GROUP BY tag ORDER BY 2 DESC LIMIT 10;

-- помилки й повільні запити за добу
SELECT status, count(*), round(avg(duration_ms)) AS avg_ms
FROM queries WHERE created_at > now() - interval '1 day' GROUP BY status;

-- клієнти
SELECT client_os, client_browser, count(*) FROM queries GROUP BY 1, 2 ORDER BY 3 DESC;
```

## Docker

```sh
podman build -t overpass-history .
podman run --rm -p 8081:8081 -e DATABASE_URL=postgres://... overpass-history
```

Образ: distroless `cc-debian13:nonroot`. Healthcheck робиться ззовні через `/healthz`.

У `compose.yaml` облікові дані Postgres за замовчуванням `history`/`history`. Перевизначаються змінними `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` (або через `.env` поруч із `compose.yaml`). Дані лежать у volume `pgdata`. Видалити разом із даними: `podman-compose down -v`.
