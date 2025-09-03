import app from "./app";
import { checkUsersPath } from "./utils/path";
import { env } from "./validation/envSchema";

async function startServer() {
  try {
    await checkUsersPath();
    app.listen(env.PORT, () => {
      console.log(`Server initialized on port ${env.PORT}`);
      console.log(`USERS_PATH: ${env.USERS_PATH}`);
    });
  } catch (err) {
    console.error(err instanceof Error ? err.message : err);
    process.exit(1);
  }
}

startServer();
