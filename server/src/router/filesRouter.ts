import { Router, Request, Response, NextFunction } from "express";
import { StatusCodes } from "http-status-codes";
import fs from "fs/promises";
import multiparty from "multiparty";
import path from "path";
import { FileAttr, getNodeType } from "../model/file";
import { FileError } from "../error/fileError";

export const filesRouter: Router = Router();

const USER_PATH = process.env.USER_PATH;

// GET /files/:path
filesRouter.get(
  "/files/:path?",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      if (!req.params.path || req.params.path.includes(".."))
        return next(FileError.InvalidPath());

      const filePath = `${USER_PATH}${req.params.path}`;
      const fileContent = await fs.readFile(filePath);
      res.status(StatusCodes.OK).send(fileContent);
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else {
        next(e);
      }
    }
  }
);

// PUT /files/:path
filesRouter.put("/files/:path", async (req: Request, res: Response) => {
  const currentPath = req.params.path;

  try {
    const form = new multiparty.Form();
    form.parse(req, async (err, fields, files) => {
      if (err || !fields || !files || !files.content) {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Invalid multipart data" });
        return;
      }

      let metadata;
      try {
        metadata = JSON.parse(fields.metadata?.[0] || "{}");
      } catch {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Malformed metadata JSON" });
        return;
      }

      const { newPath, size, permissions, lastModified } = metadata;
      if (!size || !permissions || !lastModified) {
        // TODO: make this a FileError
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Missing required metadata fields" });
        return;
      }

      const finalPath = `${USER_PATH}${newPath || currentPath}`;

      const contentFile = files.content[0];
      console.log(contentFile.path);
      const fileBuffer = await fs.readFile(contentFile.path);

      // Verify integrity
      if (fileBuffer.length !== size) {
        res
          .status(StatusCodes.BAD_REQUEST) // TODO: make this a FileError
          .send({ error: "Size mismatch: integrity verification failed" });
        return;
      }

      await fs.mkdir(path.dirname(finalPath), { recursive: true });

      let fileAlreadyExists = false;
      try {
        await fs.access(finalPath);
        fileAlreadyExists = true;
      } catch {
        /* empty */
      }

      await fs.writeFile(finalPath, fileBuffer);

      // Attempt to set last modified timestamp // TODO: this should work when there are sync problems
      /*
      try {
        await fs.utimes(finalPath, new Date(), new Date(modified));
      } catch {
        // best-effort only
      }
        */

      // Handle move: remove original if path differs
      if (newPath && newPath !== currentPath) {
        const oldPath = `${USER_PATH}${currentPath}`;
        if (oldPath !== finalPath) {
          try {
            await fs.rm(oldPath);
          } catch {
            // file may already have been overwritten
          }
        }
        res.status(StatusCodes.NO_CONTENT).send(); // If the file was moved to a new path or its content was updated
      } else {
        const status = fileAlreadyExists
          ? StatusCodes.NO_CONTENT
          : StatusCodes.CREATED;
        res.status(status).send();
      }
    });
  } catch (error) {
    res
      .status(StatusCodes.INTERNAL_SERVER_ERROR) // TODO: Make this a FileError
      .send({ error: "Unable to write file", description: error });
  }
});

// DELETE /files/:path
filesRouter.delete(
  "/files/:path?",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      if (!req.params.path || req.params.path.includes(".."))
        return next(FileError.InvalidPath());

      const filePath = `${USER_PATH}${req.params.path}`;
      const stat = await fs.stat(filePath);

      if (stat.isDirectory()) {
        await fs.rmdir(filePath);
      } else {
        await fs.unlink(filePath);
      }

      res.status(StatusCodes.NO_CONTENT).send();
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else if (code === "ENOTEMPTY") {
        next(FileError.DirectoryNotEmpty());
      } else {
        next(e);
      }
    }
  }
);

// GET /list/:path
filesRouter.get(
  "/list/:path?",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      if (req.params.path && req.params.path.includes(".."))
        return next(FileError.InvalidPath());

      const entryPath = `${USER_PATH}${req.params.path ?? ""}`;

      const stats = await fs.stat(entryPath);

      // if the entry is a file, the output will be an array with a single object containing its metadata
      if (!stats.isDirectory()) {
        const parentDirPath = path.dirname(entryPath);
        const entries = await fs.readdir(parentDirPath, {
          withFileTypes: true,
        });
        const entry = entries.find((e) => e.name === path.basename(entryPath))!; // cannot be undefined
        const fsEntry: FileAttr = {
          name: entry.name,
          size: stats.size,
          atime: stats.atime.toISOString(),
          mtime: stats.mtime.toISOString(),
          ctime: stats.ctime.toISOString(),
          crtime: stats.birthtime.toISOString(),
          kind: getNodeType(entry),
          perm: (stats.mode & 0o777).toString(8), // octal mask to isolate permissions bits
          nlink: stats.nlink,
        };

        return res.status(StatusCodes.OK).json([fsEntry]);
      }

      const dirPath = entryPath; // the entry is now assumed to be a directory
      const entries = await fs.readdir(dirPath, { withFileTypes: true });

      const result = await Promise.all(
        entries.map(async (entry) => {
          const entryPath = path.join(dirPath, entry.name);
          const stats = await fs.stat(entryPath);
          return {
            name: entry.name,
            size: stats.size,
            atime: stats.atime.toISOString(),
            mtime: stats.mtime.toISOString(),
            ctime: stats.ctime.toISOString(),
            crtime: stats.birthtime.toISOString(),
            kind: getNodeType(entry),
            perm: (stats.mode & 0o777).toString(8),
            nlink: stats.nlink,
          } satisfies FileAttr;
        })
      );

      res.status(StatusCodes.OK).json(result);
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else {
        next(e);
      }
    }
  }
);

// POST /mkdir/:path
filesRouter.post(
  "/mkdir/:path?",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      if (!req.params.path || req.params.path.includes(".."))
        return next(FileError.InvalidPath());

      const dirPath = `${USER_PATH}${req.params.path}`;

      await fs.mkdir(dirPath);
      res.status(StatusCodes.CREATED).send();
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT")
        next(FileError.NotFound("Parent directory does not exist"));
      else if (code === "EEXIST") {
        next(FileError.DirectoryAlreadyExists());
      } else {
        next(e);
      }
    }
  }
);
