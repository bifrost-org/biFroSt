import express from "express";
import morgan from "morgan";
import { sinkErrorHandler } from "./middleware/error";
import { filesRouter } from "./router/filesRouter";
import { usersRouter } from "./router/usersRouter";
import { checkUsersPath } from "./utils/path";
const app = express();

// Middlewares
app.use(express.json());
app.use(morgan("dev"));
app.use((_req, _res, next) => {
  checkUsersPath()
    .then(() => next())
    .catch(next);
});

// Routes
app.use("", filesRouter);
app.use("/users", usersRouter);

// Error handler middleware. Do not move
app.use(sinkErrorHandler);

export default app;
