package com.dmetasoul.lakesoul.meta.dao;

import com.dmetasoul.lakesoul.meta.DBConnector;
import com.dmetasoul.lakesoul.meta.DBUtil;
import com.dmetasoul.lakesoul.meta.entity.TableInfo;
import org.apache.commons.lang.StringUtils;

import java.sql.*;

public class TableInfoDao {

    public TableInfo selectByTableId(String tableId) {

        String sql = String.format("select * from table_info where table_id = '%s'", tableId);
        return getTableInfo(sql);
    }

    public TableInfo selectByTableName(String tableName) {
        String sql = String.format("select * from table_info where table_name = '%s'", tableName);
        return getTableInfo(sql);
    }

    public TableInfo selectByTablePath(String tablePath) {
        String sql = String.format("select * from table_info where table_path = '%s'", tablePath);
        return getTableInfo(sql);
    }

    public TableInfo selectByIdAndTablePath(String tableId, String tablePath) {
        String sql = String.format("select * from table_info where table_id = '%s' and table_path = '%s' ", tableId, tablePath);
        return getTableInfo(sql);
    }

    public TableInfo selectByIdAndTableName(String tableId, String tableName) {
        String sql = String.format("select * from table_info where table_id = '%s' and table_name = '%s' ", tableId, tableName);
        return getTableInfo(sql);
    }

    private TableInfo getTableInfo(String sql) {
        Connection conn = null;
        PreparedStatement pstmt = null;
        ResultSet rs = null;
        TableInfo tableInfo = null;
        try {
            conn = DBConnector.getConn();
            pstmt = conn.prepareStatement(sql);
            rs = pstmt.executeQuery();
            while (rs.next()) {
                tableInfo = new TableInfo();
                tableInfo.setTableId(rs.getString("table_id"));
                tableInfo.setTableName(rs.getString("table_name"));
                tableInfo.setTablePath(rs.getString("table_path"));
                tableInfo.setTableSchema(rs.getString("table_schema"));
                tableInfo.setProperties(DBUtil.stringToJSON(rs.getString("properties")));
                tableInfo.setPartitions(rs.getString("partitions"));
            }
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            DBConnector.closeConn(rs, pstmt, conn);
        }
        return tableInfo;
    }

    public boolean insert(TableInfo tableInfo) {
        Connection conn = null;
        PreparedStatement pstmt = null;
        boolean result = true;
        try {
            conn = DBConnector.getConn();
            pstmt = conn.prepareStatement("insert into table_info(table_id, table_name, table_path, table_schema, properties, partitions) " +
                    "values (?, ?, ?, ?, ?, ?)");
            pstmt.setString(1, tableInfo.getTableId());
            pstmt.setString(2, tableInfo.getTableName());
            pstmt.setString(3, tableInfo.getTablePath());
            pstmt.setString(4, tableInfo.getTableSchema());
            pstmt.setString(5, DBUtil.jsonToString(tableInfo.getProperties()));
            pstmt.setString(6, tableInfo.getPartitions());
            pstmt.execute();
        } catch (SQLException e) {
            result = false;
            e.printStackTrace();
        } finally {
            DBConnector.closeConn(pstmt, conn);
        }
        return result;
    }

    public void deleteByTableId(String tableId) {
        Connection conn = null;
        PreparedStatement pstmt = null;
        String sql = String.format("delete from table_info where table_id = '%s' ", tableId);
        try {
            conn = DBConnector.getConn();
            pstmt = conn.prepareStatement(sql);
            pstmt.execute();
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            DBConnector.closeConn(pstmt, conn);
        }
    }

    public void deleteByIdAndPath(String tableId, String tablePath) {
        Connection conn = null;
        PreparedStatement pstmt = null;
        String sql = String.format("delete from table_info where table_id = '%s' and table_path = '%s'", tableId, tablePath);
        try {
            conn = DBConnector.getConn();
            pstmt = conn.prepareStatement(sql);
            pstmt.execute();
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            DBConnector.closeConn(pstmt, conn);
        }
    }

    public int updateByTableId(String tableId, String tableName, String tablePath, String tableSchema) {
        int result = 0;
        if (StringUtils.isBlank(tableName) && StringUtils.isBlank(tablePath) && StringUtils.isBlank(tableSchema)) {
            return result;
        }
        Connection conn = null;
        PreparedStatement pstmt = null;
        StringBuilder sb = new StringBuilder();
        sb.append("update table_info set ");
        if (StringUtils.isNotBlank(tableName)) {
            sb.append(String.format("table_name = '%s', ", tableName));
        }
        if (StringUtils.isNotBlank(tablePath)) {
            sb.append(String.format("table_path = '%s', ", tablePath));
        }
        if (StringUtils.isNotBlank(tableSchema)) {
            sb.append(String.format("table_schema = '%s', ", tableSchema));
        }
        sb = new StringBuilder(sb.substring(0, sb.length()-2));
        sb.append(String.format(" where table_id = '%s'", tableId));
        try {
            conn = DBConnector.getConn();
            pstmt = conn.prepareStatement(sb.toString());
            result = pstmt.executeUpdate();
        } catch (SQLException e) {
            e.printStackTrace();
        } finally {
            DBConnector.closeConn(pstmt, conn);
        }
        return result;
    }
}
