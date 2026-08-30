PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description TEXT NOT NULL DEFAULT '',
    color       TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    archived_at INTEGER
);

CREATE TABLE IF NOT EXISTS tasks (
    id            TEXT PRIMARY KEY NOT NULL,
    title         TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 500),
    memo          TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'todo'
                  CHECK (status IN ('todo', 'doing', 'blocked', 'done', 'archived')),
    priority      TEXT NOT NULL DEFAULT 'none'
                  CHECK (priority IN ('none', 'low', 'medium', 'high')),
    progress      INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    due_kind      TEXT NOT NULL DEFAULT 'none'
                  CHECK (due_kind IN ('none', 'date', 'datetime')),
    due_date      TEXT,
    due_at        INTEGER,
    project_id    TEXT REFERENCES projects(id) ON DELETE SET NULL,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    completed_at  INTEGER,
    deleted_at    INTEGER,
    CHECK (
        (due_kind = 'none' AND due_date IS NULL AND due_at IS NULL) OR
        (due_kind = 'date' AND due_date IS NOT NULL AND due_at IS NULL) OR
        (due_kind = 'datetime' AND due_date IS NULL AND due_at IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS tags (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL COLLATE NOCASE UNIQUE
                CHECK (length(name) BETWEEN 1 AND 50),
    color       TEXT,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS task_tags (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    tag_id  TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, tag_id)
);

CREATE TABLE IF NOT EXISTS saved_views (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL COLLATE NOCASE UNIQUE
                CHECK (length(name) BETWEEN 1 AND 100),
    view_kind   TEXT NOT NULL
                CHECK (view_kind IN ('list', 'board', 'calendar')),
    filter_json TEXT NOT NULL DEFAULT '{}',
    sort_json   TEXT NOT NULL DEFAULT '[]',
    group_by    TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_due_date ON tasks(due_date) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_due_at ON tasks(due_at) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_project_id ON tasks(project_id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_updated_at ON tasks(updated_at) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_deleted_at ON tasks(deleted_at);
CREATE INDEX IF NOT EXISTS idx_task_tags_tag_id ON task_tags(tag_id);
CREATE INDEX IF NOT EXISTS idx_projects_archived_at ON projects(archived_at);
CREATE INDEX IF NOT EXISTS idx_saved_views_sort_order ON saved_views(sort_order);

PRAGMA user_version = 2;
