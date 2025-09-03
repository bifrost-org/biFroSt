import app from "./app";
import { env } from "./validation/envSchema";

export const server = app.listen(env.PORT, () => {});

console.log(`Server inizialized on port ${env.PORT}`);
