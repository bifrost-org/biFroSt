import * as dotenv from "dotenv";
import app from "./app";

dotenv.config();

const port = process.env.PORT ? parseInt(process.env.PORT) : 3000; // default port 3000

export const server = app.listen(port, () => {});

console.log(`Server inizialized on port ${port}`);
