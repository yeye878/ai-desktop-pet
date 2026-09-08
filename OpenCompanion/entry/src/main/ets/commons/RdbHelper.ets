import { relationalStore } from '@kit.ArkData';
import { AppContext } from './AppContext';
import { Log } from './Log';

/**
 * 主数据库（对应桌面版 storage/db.rs 的 SQLite schema，鸿蒙用 relationalStore 复用同构表）
 */
export class RdbHelper {
  private static store: relationalStore.RdbStore | null = null;

  static getConfig(): relationalStore.StoreConfig {
    return {
      name: 'companion.db',
      securityLevel: relationalStore.SecurityLevel.S1
    } as relationalStore.StoreConfig;
  }

  static async init(): Promise<void> {
    if (RdbHelper.store) {
      return;
    }
    const ctx = AppContext.getContext();
    const store = await relationalStore.getRdbStore(ctx, RdbHelper.getConfig());
    RdbHelper.store = store;
    const ddls: string[] = [
      `CREATE TABLE IF NOT EXISTS chat_history (
        id TEXT PRIMARY KEY, role TEXT NOT NULL, content TEXT NOT NULL, thinking TEXT DEFAULT '',
        agent_name TEXT DEFAULT '', agent_avatar TEXT DEFAULT '', tool_events TEXT DEFAULT '[]',
        image_uris TEXT DEFAULT '[]', quote TEXT DEFAULT '', created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS pet_memory (
        id TEXT PRIMARY KEY, category TEXT DEFAULT 'general', key TEXT DEFAULT '',
        value TEXT DEFAULT '', created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS scheduled_tasks (
        id TEXT PRIMARY KEY, title TEXT NOT NULL, note TEXT DEFAULT '', due_at INTEGER,
        repeat TEXT DEFAULT 'once', enabled INTEGER DEFAULT 1, last_triggered_at INTEGER DEFAULT 0,
        created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS agents (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, avatar TEXT DEFAULT '🤖', description TEXT DEFAULT '',
        system_prompt TEXT DEFAULT '', allowed_tools TEXT DEFAULT '[]', is_builtin INTEGER DEFAULT 0,
        created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS skills (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT DEFAULT '',
        system_prompt TEXT DEFAULT '', allowed_tools TEXT DEFAULT '[]', is_active INTEGER DEFAULT 0)`,
      `CREATE TABLE IF NOT EXISTS evolution_insights (
        id TEXT PRIMARY KEY, category TEXT DEFAULT 'preference', content TEXT NOT NULL,
        confidence REAL DEFAULT 0.6, hit_count INTEGER DEFAULT 1, status TEXT DEFAULT 'active',
        created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS evolution_proposals (
        id TEXT PRIMARY KEY, ptype TEXT NOT NULL, name TEXT DEFAULT '', payload TEXT DEFAULT '{}',
        status TEXT DEFAULT 'pending', created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS evolution_logs (
        id TEXT PRIMARY KEY, action TEXT NOT NULL, before_state TEXT DEFAULT '{}',
        after_state TEXT DEFAULT '{}', created_at INTEGER)`,
      `CREATE TABLE IF NOT EXISTS todos (
        id TEXT PRIMARY KEY, title TEXT NOT NULL, note TEXT DEFAULT '',
        done INTEGER DEFAULT 0, created_at INTEGER)`
    ];
    for (const ddl of ddls) {
      await store.executeSql(ddl);
    }
    Log.info('rdb initialized');
  }

  static get(): relationalStore.RdbStore {
    if (!RdbHelper.store) {
      throw new Error('rdb not initialized');
    }
    return RdbHelper.store;
  }

  static async insert(table: string, row: Object): Promise<void> {
    await RdbHelper.get().insert(table, row as relationalStore.ValuesBucket);
  }

  static async query(sql: string, args: Array<relationalStore.ValueType> = []): Promise<relationalStore.ResultSet> {
    return await RdbHelper.get().querySql(sql, args);
  }

  static async exec(sql: string, args: Array<relationalStore.ValueType> = []): Promise<void> {
    await RdbHelper.get().executeSql(sql, args);
  }

  static resultSetToArray(rs: relationalStore.ResultSet): Array<Map<string, Object>> {
    const out: Array<Map<string, Object>> = [];
    try {
      while (rs.goToNextRow()) {
        const row: Map<string, Object> = new Map<string, Object>();
        const names = rs.columnNames;
        for (let i = 0; i < names.length; i++) {
          row.set(names[i], rs.getValue(rs.getColumnIndex(names[i])) as Object);
        }
        out.push(row);
      }
    } finally {
      rs.close();
    }
    return out;
  }
}
