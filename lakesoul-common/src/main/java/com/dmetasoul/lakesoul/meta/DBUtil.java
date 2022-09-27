/*
 * Copyright [2022] [DMetaSoul Team]
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 */

package com.dmetasoul.lakesoul.meta;

import com.alibaba.fastjson.JSON;
import com.alibaba.fastjson.JSONArray;
import com.alibaba.fastjson.JSONObject;
import com.dmetasoul.lakesoul.meta.entity.DataBaseProperty;
import com.dmetasoul.lakesoul.meta.entity.DataFileOp;

import java.io.FileInputStream;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.sql.Connection;
import java.sql.SQLException;
import java.sql.Statement;
import java.util.*;

public class DBUtil {

    public static DataBaseProperty getDBInfo() {
        String lakesoulhome = "lakesoul_home";
        String configFile = System.getenv(lakesoulhome);
        if (null == configFile) {
            configFile = System.getProperty(lakesoulhome);
        }
        Properties properties = new Properties();
        if (configFile != null) {
            try {
                properties.load(Files.newInputStream(Paths.get(configFile)));
            } catch (IOException e) {
                e.printStackTrace();
            }
        } else {
            properties.setProperty("lakesoul.pg.driver", System.getenv("lakesoul.pg.driver"));
            properties.setProperty("lakesoul.pg.url", System.getenv("lakesoul.pg.url"));
            properties.setProperty("lakesoul.pg.username", System.getenv("lakesoul.pg.username"));
            properties.setProperty("lakesoul.pg.password", System.getenv("lakesoul.pg.password"));
        }

        DataBaseProperty dataBaseProperty = new DataBaseProperty();
        dataBaseProperty.setDriver(properties.getProperty("lakesoul.pg.driver", "org.postgresql.Driver"));
        dataBaseProperty.setUrl(properties.getProperty(
                "lakesoul.pg.url", "jdbc:postgresql://127.0.0.1:5433/lakesoul_test?stringtype=unspecified"));
        dataBaseProperty.setUsername(properties.getProperty("lakesoul.pg.username", "lakesoul_test"));
        dataBaseProperty.setPassword(properties.getProperty("lakesoul.pg.password", "lakesoul_test"));
        for (Object key : properties.keySet()) {
            System.out.println("property---" + key + ": " + properties.getProperty(key.toString()));
        }
        return dataBaseProperty;
    }

    public static void cleanAllTable() {
        String tableInfo = "truncate table table_info";
        String tableNameId = "truncate table table_name_id";
        String tablePathId = "truncate table table_path_id";
        String dataCommitInfo = "truncate table data_commit_info";
        String partitionInfo = "truncate table partition_info";
        Connection conn;
        Statement stmt;
        try {
            conn = DBConnector.getConn();
            stmt = conn.createStatement();
            stmt.addBatch(tableInfo);
            stmt.addBatch(tableNameId);
            stmt.addBatch(tablePathId);
            stmt.addBatch(dataCommitInfo);
            stmt.addBatch(partitionInfo);
            stmt.executeBatch();
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            DBConnector.closeConn();
        }
    }

    public static JSONObject stringToJSON(String s) {
        return JSONObject.parseObject(s);
    }

    public static JSONObject stringMapToJson(Map<String, String> map) {
        JSONObject object = new JSONObject();
        object.putAll(map);
        return object;
    }

    public static Map<String, String> jsonToStringMap(JSONObject o) {
        Map<String, String> map = new HashMap<>();
        for (Map.Entry<String, Object> entry : o.entrySet()) {
            map.put(entry.getKey(), entry.getValue().toString());
        }
        return map;
    }

    public static String jsonToString(JSONObject o) {
        return JSON.toJSONString(o);
    }

    public static JSONArray stringToJSONArray(String s) {
        return JSONArray.parseArray(s);
    }

    public static String changeDataFileOpListToString(List<DataFileOp> dataFileOpList) {
        if (dataFileOpList.size() < 1) {
            return "{}";
        }
        StringBuilder sb = new StringBuilder();
        sb.append("{");
        for (DataFileOp dataFileOp : dataFileOpList) {
            String path = dataFileOp.getPath();
            String fileOp = dataFileOp.getFileOp();
            long size = dataFileOp.getSize();
            String fileExistCols = dataFileOp.getFileExistCols();
            sb.append(String.format("\"(%s,%s,%s,\\\"%s\\\")\",", path, fileOp, size, fileExistCols));
        }
        sb = new StringBuilder(sb.substring(0, sb.length() - 1));
        sb.append("}");
        return sb.toString();
    }

    public static List<DataFileOp> changeStringToDataFileOpList(String s) {
        List<DataFileOp> rsList = new ArrayList<>();
        if (!s.startsWith("{") || !s.endsWith("}")) {
            // todo 这里应该报错
            return rsList;
        }
        String[] fileOpTmp = s.substring(1, s.length() - 1).split("\",\"");
        for (String value : fileOpTmp) {
            String tmpElem = value.replace("\"", "").replace("\\", "");
            if (!tmpElem.startsWith("(") || !tmpElem.endsWith(")")) {
                // todo 报错
                continue;
            }
            tmpElem = tmpElem.substring(1, tmpElem.length() - 1);
            DataFileOp dataFileOp = new DataFileOp();
            dataFileOp.setPath(tmpElem.substring(0, tmpElem.indexOf(",")));
            tmpElem = tmpElem.substring(tmpElem.indexOf(",") + 1);
            String fileOp = tmpElem.substring(0, tmpElem.indexOf(","));
            dataFileOp.setFileOp(fileOp);
            tmpElem = tmpElem.substring(tmpElem.indexOf(",") + 1);
            dataFileOp.setSize(Long.parseLong(tmpElem.substring(0, tmpElem.indexOf(","))));
            tmpElem = tmpElem.substring(tmpElem.indexOf(",") + 1);
            dataFileOp.setFileExistCols(tmpElem);
            rsList.add(dataFileOp);
        }
        return rsList;
    }

    public static String changeUUIDListToString(List<UUID> uuidList) {
        StringBuilder sb = new StringBuilder();
        if (uuidList.size() == 0) {
            return sb.toString();
        }
        for (UUID uuid : uuidList) {
            sb.append(String.format("'%s',", uuid.toString()));
        }
        sb = new StringBuilder(sb.substring(0, sb.length() - 1));
        return sb.toString();
    }

    public static String changeUUIDListToOrderString(List<UUID> uuidList) {
        StringBuilder sb = new StringBuilder();
        if (uuidList.size() == 0) {
            return sb.toString();
        }
        for (UUID uuid : uuidList) {
            sb.append(String.format("%s,", uuid.toString()));
        }
        sb = new StringBuilder(sb.substring(0, sb.length() - 1));
        return sb.toString();
    }

    public static List<UUID> changeStringToUUIDList(String s) {
        List<UUID> uuidList = new ArrayList<>();
        if (!s.startsWith("{") || !s.endsWith("}")) {
            // todo
            return uuidList;
        }
        s = s.substring(1, s.length() - 1);
        String[] uuids = s.split(",");
        for (String uuid : uuids) {
            uuidList.add(UUID.fromString(uuid));
        }
        return uuidList;
    }

    public static String changePartitionDescListToString(List<String> partitionDescList) {
        StringBuilder sb = new StringBuilder();
        if (partitionDescList.size() < 1) {
            return sb.append("''").toString();
        }
        for (String s : partitionDescList) {
            sb.append(String.format("'%s',", s));
        }
        return sb.substring(0, sb.length() - 1);
    }

}
