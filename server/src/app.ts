import cors from "cors";
import * as dotenv from "dotenv";
import express from "express";
import session from "express-session";
import { StatusCodes } from "http-status-codes";
import morgan from "morgan";
import passport from "passport";
import { sinkErrorHandler } from "./middleware/error";
import passportInitializer from "./passport-config";
import { filesRouter } from "./router/filesRouter";

dotenv.config();

const app = express();

// Middlewares
// app.use(express.json({ limit: "15mb" }));
app.use(morgan("dev"));

const corsOptions = {
  origin: "http://localhost:5173", //what to put now? Clients can be multiple and from multiple terminals. What if the client can change port?
  optionsSuccessStatus: StatusCodes.ACCEPTED,
  credentials: true,
};
app.use(cors(corsOptions));

passportInitializer(passport);

app.use(
  session({
    secret: process.env.SESSION_SECRET!,
    resave: false,
    saveUninitialized: false,
  })
);
app.use(passport.authenticate("session"));

// Routes

app.use("", filesRouter);

// Error handler middleware. Do not move
app.use(sinkErrorHandler);

export default app;
