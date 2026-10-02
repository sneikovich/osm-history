CREATE TABLE queries (
    id             bigserial PRIMARY KEY,
    created_at     timestamptz NOT NULL DEFAULT now(),
    tags           text[]      NOT NULL,
    area_type      text        NOT NULL CHECK (area_type IN ('none', 'around', 'bbox')),
    -- around: lat,lon,radius_m; bbox: south,west,north,east; none: NULL
    coords         double precision[],
    kind           text CHECK (kind IN ('node', 'way', 'relation')),
    status         smallint    NOT NULL,
    element_count  integer,
    duration_ms    integer     NOT NULL,
    client_os      text,
    client_browser text
);

CREATE INDEX queries_created_at_idx ON queries (created_at DESC);
