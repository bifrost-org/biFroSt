import * as dotenv from "dotenv";
import express from "express";
import morgan from "morgan";
import { sinkErrorHandler } from "./middleware/error";
import { filesRouter } from "./router/filesRouter";
import { usersRouter } from "./router/usersRouter";

dotenv.config();

const app = express();

// Middlewares
app.use(express.json());
app.use(morgan("dev"));

// Routes
app.use("", filesRouter);
app.use("/users", usersRouter);

// Error handler middleware. Do not move
app.use(sinkErrorHandler);

export default app;
