UPDATE tasks SET status = 'todo' WHERE status = 'inbox';

PRAGMA user_version = 2;
