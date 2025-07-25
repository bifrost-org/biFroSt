import * as dotenv from "dotenv";
dotenv.config();

import app from "./app";

const port = process.env.PORT ? parseInt(process.env.PORT) : 3000; // default port 3000

export const server = app.listen(port, () => {});

console.log(`Server inizialized on port ${port}`);
