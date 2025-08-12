import { Router, Request, Response, NextFunction } from "express";
import User from "../model/user";
import { StatusCodes } from "http-status-codes";
import { UserError } from "../error/userError";
import { validateBody } from "../middleware/validation";
import { registrationSchema } from "../validation/userSchema";

export const usersRouter: Router = Router();

usersRouter.post(
  "/",
  validateBody(registrationSchema),
  async (req: Request, res: Response, next: NextFunction) => {
    const user = await User.register(req.body.username);

    if (!user) return next(UserError.RegistrationFailed());

    res
      .status(StatusCodes.CREATED)
      .send({ api_key: user.apiKey, secret_key: user.secretKey });
  }
);
