import * as dotenv from "dotenv";
import express from "express";
import morgan from "morgan";
import { sinkErrorHandler } from "./middleware/error";
import { filesRouter } from "./router/filesRouter";

dotenv.config();

const app = express();

// Middlewares
app.use(morgan("dev"));

// Routes
app.use("", filesRouter);

// Error handler middleware. Do not move
app.use(sinkErrorHandler);

export default app;
